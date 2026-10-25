use crate::dashboard::ServerRequest;
use alvr_gui_common::theme;
use alvr_localization::tr;
use eframe::egui::{Frame, Grid, RichText, ScrollArea, Ui};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const DRIVER_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

pub enum InstallationTabRequest {
    OpenSetupWizard,
    ServerRequest(ServerRequest),
}

pub struct InstallationTab {
    drivers: Vec<PathBuf>,
    last_update_instant: Instant,
}

impl InstallationTab {
    pub fn new() -> Self {
        Self {
            drivers: vec![],
            last_update_instant: Instant::now(),
        }
    }

    pub fn update_drivers(&mut self, list: Vec<PathBuf>) {
        self.drivers = list;
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Vec<InstallationTabRequest> {
        let mut requests = vec![];

        let now = Instant::now();
        if now > self.last_update_instant + DRIVER_UPDATE_INTERVAL {
            requests.push(InstallationTabRequest::ServerRequest(
                ServerRequest::GetDriverList,
            ));

            self.last_update_instant = now;
        }

        ui.vertical_centered_justified(|ui| {
            if ui.button(tr("Run setup wizard")).clicked() {
                requests.push(InstallationTabRequest::OpenSetupWizard);
            }
            ui.columns(2, |ui| {
                if ui[0].button(tr("Add firewall rules")).clicked() {
                    requests.push(InstallationTabRequest::ServerRequest(
                        ServerRequest::AddFirewallRules,
                    ));
                }
                if ui[1].button(tr("Remove firewall rules")).clicked() {
                    requests.push(InstallationTabRequest::ServerRequest(
                        ServerRequest::RemoveFirewallRules,
                    ));
                }
            });

            Frame::group(ui.style())
                .fill(theme::SECTION_BG)
                .inner_margin(theme::FRAME_PADDING)
                .show(ui, |ui| {
                    ui.vertical_centered_justified(|ui| {
                        ui.label(RichText::new(tr("Registered drivers")).size(18.0));
                    });

                    Grid::new(0).num_columns(2).show(ui, |ui| {
                        for driver_path in &self.drivers {
                            if ui.button(tr("Remove")).clicked() {
                                requests.push(InstallationTabRequest::ServerRequest(
                                    ServerRequest::UnregisterDriver(driver_path.clone()),
                                ));
                            }

                            ScrollArea::new([true, false])
                                .auto_shrink([false, false])
                                .id_salt(driver_path)
                                .show(ui, |ui| {
                                    ui.label(driver_path.to_string_lossy());
                                });
                            ui.end_row();
                        }
                    });

                    if ui.button(tr("Register ALVR driver")).clicked() {
                        requests.push(InstallationTabRequest::ServerRequest(
                            ServerRequest::RegisterAlvrDriver,
                        ));
                    }
                });
        });

        requests
    }
}
