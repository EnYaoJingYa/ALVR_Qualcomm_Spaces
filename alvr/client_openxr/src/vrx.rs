//! Validate requests against the enabled Spaces feature set before applying them.
use alvr_common::anyhow::{Result, ensure};
use alvr_packets::RealTimeConfig;
use alvr_session::{
    ClientsidePostProcessingConfig, ClientsidePostProcessingSharpeningMode,
    ClientsidePostProcessingSuperSamplingMode, FaceTrackingSourcesConfig, Settings,
};

fn validate_composition(
    passthrough: bool,
    post_processing: Option<&ClientsidePostProcessingConfig>,
) -> Result<()> {
    ensure!(
        !passthrough,
        "Passthrough is not implemented for the VRX Spaces runtime (FB/HTC passthrough is not enabled)"
    );
    let post_processing_enabled = post_processing.is_some_and(|config| {
        config.super_sampling != ClientsidePostProcessingSuperSamplingMode::Disabled
            || config.sharpening != ClientsidePostProcessingSharpeningMode::Disabled
    });
    ensure!(
        !post_processing_enabled,
        "Client-side compositor post-processing requires XR_FB_composition_layer_settings, which is not enabled on VRX"
    );
    Ok(())
}

pub fn validate_stream_settings(settings: &Settings) -> Result<()> {
    validate_composition(
        settings.video.passthrough.enabled(),
        settings.video.clientside_post_processing.as_option(),
    )?;
    ensure!(
        !settings.video.clientside_foveation.enabled(),
        "Device-side foveation requires XR_FB_foveation, XR_FB_foveation_configuration and XR_FB_swapchain_update_state, which are not enabled on VRX"
    );
    if let Some(face) = settings.headset.face_tracking.as_option() {
        ensure!(
            face.sources == FaceTrackingSourcesConfig::PreferEyeTrackingOnly
                && face.eye_tracking_osc_override.enabled(),
            "Native eye/face tracking is not implemented in the VRX Spaces backend; external eye-only OSC input is supported"
        );
    }
    ensure!(
        !settings.headset.body_tracking.enabled(),
        "Native body tracking is not implemented in the VRX Spaces backend"
    );
    ensure!(
        !settings
            .headset
            .multimodal_tracking
            .as_option()
            .is_some_and(|config| config.enabled || config.detached_controllers_steamvr_sink),
        "Simultaneous hand/controller and detached-controller tracking require Meta extensions that are not enabled on VRX"
    );
    Ok(())
}

pub fn validate_real_time_config(config: &RealTimeConfig) -> Result<()> {
    validate_composition(
        config.passthrough.is_some(),
        config.clientside_post_processing.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alvr_common::settings_schema::Switch;
    use alvr_session::{OscPort, SessionConfig};

    #[test]
    fn basic_stream_and_external_eye_input_do_not_require_native_eye_tracking() {
        let mut settings = SessionConfig::default().to_settings();
        assert!(validate_stream_settings(&settings).is_ok());
        let mut session = SessionConfig::default();
        session.session_settings.headset.face_tracking.enabled = true;
        let Switch::Enabled(mut face) = session.to_settings().headset.face_tracking else {
            panic!("Face tracking must be enabled in this test");
        };
        face.sources = FaceTrackingSourcesConfig::PreferEyeTrackingOnly;
        face.eye_tracking_osc_override = Switch::Enabled(OscPort { port: 9945 });
        settings.headset.face_tracking = Switch::Enabled(face);
        assert!(validate_stream_settings(&settings).is_ok());
        if let Switch::Enabled(face) = &mut settings.headset.face_tracking {
            face.eye_tracking_osc_override = Switch::Disabled;
        }
        assert!(validate_stream_settings(&settings).is_err());
    }

    #[test]
    fn runtime_post_processing_is_rejected_before_it_can_be_ignored() {
        let settings = SessionConfig::default().to_settings();
        let mut config = RealTimeConfig::from_settings(&settings);
        config.clientside_post_processing = Some(ClientsidePostProcessingConfig {
            super_sampling: ClientsidePostProcessingSuperSamplingMode::Disabled,
            sharpening: ClientsidePostProcessingSharpeningMode::Disabled,
        });
        assert!(validate_real_time_config(&config).is_ok());
        config
            .clientside_post_processing
            .as_mut()
            .unwrap()
            .sharpening = ClientsidePostProcessingSharpeningMode::Quality;
        assert!(
            validate_real_time_config(&config)
                .unwrap_err()
                .to_string()
                .contains("XR_FB_composition_layer_settings")
        );
    }

    #[test]
    fn unsupported_stream_options_report_the_requested_feature() {
        let mut session = SessionConfig::default();
        session.session_settings.video.clientside_foveation.enabled = true;
        assert!(
            validate_stream_settings(&session.to_settings())
                .unwrap_err()
                .to_string()
                .contains("XR_FB_foveation")
        );

        session.session_settings.video.clientside_foveation.enabled = false;
        session.session_settings.video.passthrough.enabled = true;
        assert!(
            validate_stream_settings(&session.to_settings())
                .unwrap_err()
                .to_string()
                .contains("Passthrough")
        );
        let realtime = RealTimeConfig::from_settings(&session.to_settings());
        assert!(
            validate_real_time_config(&realtime)
                .unwrap_err()
                .to_string()
                .contains("Passthrough")
        );

        session.session_settings.video.passthrough.enabled = false;
        session.session_settings.headset.body_tracking.enabled = true;
        assert!(
            validate_stream_settings(&session.to_settings())
                .unwrap_err()
                .to_string()
                .contains("body tracking")
        );
    }
}
