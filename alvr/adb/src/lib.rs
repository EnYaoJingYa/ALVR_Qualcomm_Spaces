pub mod commands;
mod parse;

use alvr_common::anyhow::Result;
use alvr_common::{dbg_connection, error, parking_lot::Mutex, warn};
use alvr_session::WiredClientAutoLaunchConfig;
use alvr_system_info::{
    ClientFlavor, PACKAGE_NAME_GITHUB_DEV, PACKAGE_NAME_GITHUB_STABLE, PACKAGE_NAME_STORE,
};
use std::collections::HashSet;
use std::time::Duration;

pub enum WiredConnectionStatus {
    Ready,
    NotReady(String),
}

pub struct WiredConnection {
    adb_path: String,
    vrx_launches: Mutex<VrxLaunchState>,
}

// A running VRX app or a successful auto-launch consumes the launch opportunity
// until USB disconnects or the user disables/re-enables auto-launch.
#[derive(Default)]
struct VrxLaunchState {
    enabled: bool,
    observed: HashSet<(String, String)>,
}

fn is_online_nonloopback_device(device: &parse::Device) -> bool {
    matches!(
        device.connection_state,
        Some(parse::ConnectionState::Device)
    ) && device
        .serial
        .as_ref()
        .is_some_and(|serial| !serial.starts_with("127.0.0.1"))
}

fn select_wired_device(devices: &[parse::Device]) -> Option<&parse::Device> {
    devices
        .iter()
        .filter(|device| is_online_nonloopback_device(device))
        .find(|device| device.model.as_deref() == Some("VRX"))
        .or_else(|| {
            devices
                .iter()
                .find(|device| is_online_nonloopback_device(device))
        })
}

impl VrxLaunchState {
    fn update_devices(&mut self, serials: &HashSet<&str>, enabled: bool) {
        self.enabled = enabled;
        self.observed
            .retain(|(serial, _)| enabled && serials.contains(serial.as_str()));
    }

    fn was_observed(&self, serial: &str, package: &str) -> bool {
        self.observed
            .contains(&(serial.to_owned(), package.to_owned()))
    }

    fn observe(&mut self, serial: &str, package: &str) {
        if self.enabled {
            self.observed
                .insert((serial.to_owned(), package.to_owned()));
        }
    }
}

impl WiredConnection {
    pub fn new(
        layout: &alvr_filesystem::Layout,
        download_progress_callback: impl Fn(usize, Option<usize>),
    ) -> Result<Self> {
        let adb_path = commands::require_adb(layout, download_progress_callback)?;

        Ok(Self {
            adb_path,
            vrx_launches: Mutex::default(),
        })
    }

    pub fn setup(
        &self,
        control_port: u16,
        stream_port: u16,
        client_type: &ClientFlavor,
        client_autolaunch: Option<WiredClientAutoLaunchConfig>,
    ) -> Result<WiredConnectionStatus> {
        let devices = commands::list_devices(&self.adb_path)?;
        self.vrx_launches.lock().update_devices(
            &devices
                .iter()
                .filter(|d| matches!(d.connection_state, Some(parse::ConnectionState::Device)))
                .filter_map(|d| d.serial.as_deref())
                .collect(),
            client_autolaunch.is_some(),
        );
        let Some(device) = select_wired_device(&devices) else {
            return Ok(WiredConnectionStatus::NotReady(
                "No wired devices found".to_owned(),
            ));
        };
        let device_serial = device.serial.as_ref().unwrap();
        let is_vrx = device.model.as_deref() == Some("VRX");
        if is_vrx {
            commands::configure_vrx_low_latency(&self.adb_path, device_serial)?;
        }

        let ports = HashSet::from([control_port, stream_port]);
        let forwarded_ports: HashSet<u16> =
            commands::list_forwarded_ports(&self.adb_path, &device_serial)?
                .into_iter()
                .map(|f| f.local)
                .collect();
        let missing_ports = ports.difference(&forwarded_ports);
        for port in missing_ports {
            commands::forward_port(&self.adb_path, &device_serial, *port)?;
            dbg_connection!(
                "setup_wired_connection: Forwarded port {port} of device {device_serial}"
            );
        }

        let Some(process_name) = get_process_name(&self.adb_path, &device_serial, client_type)
        else {
            return Ok(WiredConnectionStatus::NotReady(
                "No suitable ALVR client is installed".to_owned(),
            ));
        };

        if commands::get_process_id(&self.adb_path, &device_serial, &process_name)?.is_none() {
            if let Some(client_autolaunch) = client_autolaunch {
                if is_vrx
                    && self
                        .vrx_launches
                        .lock()
                        .was_observed(device_serial, &process_name)
                {
                    return Ok(WiredConnectionStatus::NotReady(
                        "ALVR was closed on VRX; open it in the headset to reconnect".to_owned(),
                    ));
                }
                if client_autolaunch.boot_delay > 0 {
                    match commands::get_uptime(&self.adb_path, &device_serial) {
                        Ok(uptime) => {
                            if uptime < Duration::from_secs(client_autolaunch.boot_delay.into()) {
                                return Ok(WiredConnectionStatus::NotReady(
                                    "Waiting for device boot".to_owned(),
                                ));
                            }
                        }
                        Err(failure) => {
                            warn!("wired_connection: get_uptime failed with {}", failure);
                        }
                    }
                }

                if is_vrx {
                    commands::start_vrx_application(&self.adb_path, &device_serial, &process_name)?;
                    self.vrx_launches
                        .lock()
                        .observe(device_serial, &process_name);
                } else {
                    commands::start_application(&self.adb_path, &device_serial, &process_name)?;
                }
                Ok(WiredConnectionStatus::NotReady(
                    "Starting ALVR client".to_owned(),
                ))
            } else {
                Ok(WiredConnectionStatus::NotReady(
                    "ALVR client is not running".to_owned(),
                ))
            }
        } else {
            if is_vrx {
                self.vrx_launches
                    .lock()
                    .observe(device_serial, &process_name);
            }
            if !commands::is_activity_resumed(&self.adb_path, &device_serial, &process_name)? {
                Ok(WiredConnectionStatus::NotReady(
                    "ALVR client is paused".to_owned(),
                ))
            } else {
                Ok(WiredConnectionStatus::Ready)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device(serial: &str, model: &str, state: parse::ConnectionState) -> parse::Device {
        parse::Device {
            connection_state: Some(state),
            device: Some("kona".into()),
            model: Some(model.into()),
            product: Some("kona".into()),
            serial: Some(serial.into()),
            transport_type: Some(parse::TransportType::Usb),
        }
    }

    #[test]
    fn wired_device_selection_prefers_the_online_vrx() {
        let devices = [
            test_device("192.168.1.13:5555", "LFQ1", parse::ConnectionState::Device),
            test_device("PS0012P0", "VRX", parse::ConnectionState::Device),
        ];

        assert_eq!(
            select_wired_device(&devices).unwrap().serial.as_deref(),
            Some("PS0012P0")
        );
    }

    #[test]
    fn wired_device_selection_does_not_choose_an_offline_vrx() {
        let devices = [
            test_device("PS0012P0", "VRX", parse::ConnectionState::Offline),
            test_device("192.168.1.13:5555", "LFQ1", parse::ConnectionState::Device),
        ];

        assert_eq!(
            select_wired_device(&devices).unwrap().serial.as_deref(),
            Some("192.168.1.13:5555")
        );
    }

    #[test]
    fn vrx_exit_is_respected_until_disconnect_or_autolaunch_toggle() {
        let mut state = VrxLaunchState::default();
        let devices = HashSet::from(["PS0012P0", "other"]);
        state.update_devices(&devices, true);
        assert!(!state.was_observed("PS0012P0", "alvr.client.dev"));
        state.observe("PS0012P0", "alvr.client.dev");
        state.update_devices(&devices, true);
        assert!(state.was_observed("PS0012P0", "alvr.client.dev"));
        assert!(!state.was_observed("other", "alvr.client.dev"));
        state.update_devices(&HashSet::from(["other"]), true);
        assert!(!state.was_observed("PS0012P0", "alvr.client.dev"));
        state.observe("PS0012P0", "alvr.client.dev");
        state.update_devices(&devices, false);
        state.update_devices(&devices, true);
        assert!(!state.was_observed("PS0012P0", "alvr.client.dev"));
    }

    #[test]
    fn running_vrx_does_not_consume_a_launch_while_autolaunch_is_disabled() {
        let mut state = VrxLaunchState::default();
        let devices = HashSet::from(["PS0012P0"]);
        state.update_devices(&devices, true);
        state.observe("PS0012P0", "alvr.client.dev");

        // setup() still sees the running/paused process after updating the
        // disabled setting, and observes it again in that same call.
        state.update_devices(&devices, false);
        state.observe("PS0012P0", "alvr.client.dev");
        assert!(!state.was_observed("PS0012P0", "alvr.client.dev"));

        // The process has now exited and the user explicitly enables launch.
        state.update_devices(&devices, true);
        assert!(!state.was_observed("PS0012P0", "alvr.client.dev"));
        state.observe("PS0012P0", "alvr.client.dev");
        assert!(state.was_observed("PS0012P0", "alvr.client.dev"));
    }
}

impl Drop for WiredConnection {
    fn drop(&mut self) {
        dbg_connection!("wired_connection: Killing ADB server");
        if let Err(e) = commands::kill_server(&self.adb_path) {
            error!("{e:?}");
        }
    }
}

pub fn get_process_name(
    adb_path: &str,
    device_serial: &str,
    flavor: &ClientFlavor,
) -> Option<String> {
    let fallbacks = match flavor {
        ClientFlavor::Store => {
            if alvr_common::is_stable() {
                vec![PACKAGE_NAME_STORE, PACKAGE_NAME_GITHUB_STABLE]
            } else {
                vec![PACKAGE_NAME_GITHUB_DEV]
            }
        }
        ClientFlavor::Github => {
            if alvr_common::is_stable() {
                vec![PACKAGE_NAME_GITHUB_STABLE, PACKAGE_NAME_STORE]
            } else {
                vec![PACKAGE_NAME_GITHUB_DEV]
            }
        }
        ClientFlavor::Custom(name) => {
            if alvr_common::is_stable() {
                vec![name, PACKAGE_NAME_STORE, PACKAGE_NAME_GITHUB_STABLE]
            } else {
                vec![name, PACKAGE_NAME_GITHUB_DEV]
            }
        }
    };

    fallbacks
        .iter()
        .find(|name| {
            commands::is_package_installed(adb_path, device_serial, name)
                .is_ok_and(|installed| installed)
        })
        .map(|name| (*name).to_string())
}
