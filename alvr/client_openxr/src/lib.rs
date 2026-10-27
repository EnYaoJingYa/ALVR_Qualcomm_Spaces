mod c_api;
mod extra_extensions;
mod graphics;
mod interaction;
mod lobby;
mod passthrough;
mod stream;
mod vrx;

use crate::stream::ParsedStreamConfig;
use alvr_client_core::{ClientCapabilities, ClientCoreContext, ClientCoreEvent};
use alvr_common::{
    Fov, HAND_LEFT_ID, Pose, RelaxedAtomic, debug, error,
    glam::{Quat, UVec2, Vec3},
    info,
    parking_lot::RwLock,
};
use alvr_graphics::GraphicsContext;
use alvr_localization::Language;
use alvr_session::{BodyTrackingBDConfig, BodyTrackingSourcesConfig, PerformanceLevel};
use alvr_system_info::Platform;
use extra_extensions::{
    BD_MOTION_TRACKING_EXTENSION_NAME, META_BODY_TRACKING_FIDELITY_EXTENSION_NAME,
    PICO_CONFIGURATION_EXTENSION_NAME,
};
use interaction::{InteractionContext, InteractionSourcesConfig};
use lobby::Lobby;
use openxr as xr;
use passthrough::PassthroughLayer;
use std::{ffi::CStr, path::Path, rc::Rc, sync::Arc, thread, time::Duration};
use stream::StreamContext;

const VRX_MAX_REFRESH_RATE_HZ: f32 = 90.0;

fn vrx_extensions() -> xr::ExtensionSet {
    let mut extensions = xr::ExtensionSet::default();
    #[cfg(target_os = "android")]
    {
        extensions.khr_android_create_instance = true;
        extensions.khr_android_thread_settings = true;
    }
    extensions.ext_performance_settings = true;
    extensions.fb_display_refresh_rate = true;
    extensions.khr_convert_timespec_time = true;
    extensions.khr_opengl_es_enable = true;
    extensions.other = [b"XR_QCOM_component_versioning\0".to_vec()].into();
    extensions
}

fn from_xr_vec3(v: xr::Vector3f) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn to_xr_vec3(v: Vec3) -> xr::Vector3f {
    xr::Vector3f {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

fn from_xr_quat(q: xr::Quaternionf) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

fn to_xr_quat(q: Quat) -> xr::Quaternionf {
    xr::Quaternionf {
        x: q.x,
        y: q.y,
        z: q.z,
        w: q.w,
    }
}

fn from_xr_pose(p: xr::Posef) -> Pose {
    Pose {
        orientation: from_xr_quat(p.orientation),
        position: from_xr_vec3(p.position),
    }
}

fn to_xr_pose(p: Pose) -> xr::Posef {
    xr::Posef {
        orientation: to_xr_quat(p.orientation),
        position: to_xr_vec3(p.position),
    }
}

fn from_xr_fov(f: xr::Fovf) -> Fov {
    Fov {
        left: f.angle_left,
        right: f.angle_right,
        up: f.angle_up,
        down: f.angle_down,
    }
}

fn to_xr_fov(f: Fov) -> xr::Fovf {
    xr::Fovf {
        angle_left: f.left,
        angle_right: f.right,
        angle_up: f.up,
        angle_down: f.down,
    }
}

fn from_xr_time(timestamp: xr::Time) -> Duration {
    Duration::from_nanos(timestamp.as_nanos() as _)
}

fn to_xr_time(timestamp: Duration) -> xr::Time {
    xr::Time::from_nanos(timestamp.as_nanos() as _)
}

fn to_perf_settings_level(level: PerformanceLevel) -> xr::PerfSettingsLevelEXT {
    match level {
        PerformanceLevel::PowerSavings => xr::PerfSettingsLevelEXT::POWER_SAVINGS,
        PerformanceLevel::SustainedLow => xr::PerfSettingsLevelEXT::SUSTAINED_LOW,
        PerformanceLevel::SustainedHigh => xr::PerfSettingsLevelEXT::SUSTAINED_HIGH,
        PerformanceLevel::Boost => xr::PerfSettingsLevelEXT::BOOST,
    }
}

fn set_performance_level(
    xr_instance: &xr::Instance,
    xr_session: &xr::Session<xr::OpenGlEs>,
    domain: xr::PerfSettingsDomainEXT,
    level: PerformanceLevel,
    platform: Platform,
) {
    if platform == Platform::ThinkRealityVRX {
        set_vrx_performance_level(xr_instance, xr_session, domain, level);
        return;
    }
    if let Some(performance_settings) = xr_instance.exts().ext_performance_settings {
        unsafe {
            (performance_settings.perf_settings_set_performance_level)(
                xr_session.as_raw(),
                domain,
                to_perf_settings_level(level),
            );
        }
    }
}

fn set_vrx_performance_level(
    xr_instance: &xr::Instance,
    xr_session: &xr::Session<xr::OpenGlEs>,
    domain: xr::PerfSettingsDomainEXT,
    level: PerformanceLevel,
) {
    let performance_settings = xr_instance
        .exts()
        .ext_performance_settings
        .expect("ThinkReality VRX must support XR_EXT_performance_settings");
    let result = unsafe {
        (performance_settings.perf_settings_set_performance_level)(
            xr_session.as_raw(),
            domain,
            to_perf_settings_level(level),
        )
    };

    if result != xr::sys::Result::SUCCESS {
        panic!("Qualcomm performance-level request failed for {domain:?}: {result:?}");
    }
}

fn configure_vrx_runtime(xr_instance: &xr::Instance, xr_session: &xr::Session<xr::OpenGlEs>) {
    set_vrx_performance_level(
        xr_instance,
        xr_session,
        xr::PerfSettingsDomainEXT::CPU,
        PerformanceLevel::SustainedHigh,
    );
    set_vrx_performance_level(
        xr_instance,
        xr_session,
        xr::PerfSettingsDomainEXT::GPU,
        PerformanceLevel::SustainedHigh,
    );
}

fn default_view() -> xr::View {
    xr::View {
        pose: xr::Posef {
            orientation: xr::Quaternionf {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            position: xr::Vector3f::default(),
        },
        fov: xr::Fovf {
            angle_left: -1.0,
            angle_right: 1.0,
            angle_up: 1.0,
            angle_down: -1.0,
        },
    }
}

// This exists to circumvent dead-code analysis
fn create_session(
    xr_instance: &xr::Instance,
    xr_system: xr::SystemId,
    graphics_context: &GraphicsContext,
) -> (
    xr::Session<xr::OpenGlEs>,
    xr::FrameWaiter,
    xr::FrameStream<xr::OpenGlEs>,
) {
    #[allow(unreachable_code)]
    unsafe {
        xr_instance
            .create_session(xr_system, &graphics::session_create_info(graphics_context))
            .unwrap()
    }
}

pub fn entry_point() {
    run(&RelaxedAtomic::new(false));
}

fn run(shutdown: &RelaxedAtomic) -> Platform {
    alvr_client_core::init_logging();

    const LEGACY_OPENXR_VERSION: xr::Version = xr::Version::new(1, 0, 34);
    const CURRENT_OPENXR_VERSION: xr::Version = xr::Version::new(1, 1, 36);

    // Using a provisional platform, before we can get the runtime info
    let provisional_platform = alvr_system_info::platform(None, None);
    let (loader_suffix, openxr_version) = match provisional_platform {
        Platform::Quest1 => ("_quest1", LEGACY_OPENXR_VERSION),
        Platform::PicoNeo3
        | Platform::PicoG3
        | Platform::Pico4
        | Platform::Pico4Pro
        | Platform::Pico4Enterprise => ("_pico_old", LEGACY_OPENXR_VERSION),
        p if p.is_vive() => ("", LEGACY_OPENXR_VERSION),
        p if p.is_yvr() => ("_yvr", LEGACY_OPENXR_VERSION),
        Platform::ThinkRealityVRX => ("_spaces", LEGACY_OPENXR_VERSION),
        _ => ("", CURRENT_OPENXR_VERSION),
    };
    let xr_entry = unsafe {
        xr::Entry::load_from(Path::new(&format!("libopenxr_loader{loader_suffix}.so"))).unwrap()
    };

    #[cfg(target_os = "android")]
    xr_entry.initialize_android_loader().unwrap();

    let available_exts = xr_entry.enumerate_extensions().unwrap();
    info!(
        "OpenXR available extensions: {:#?}",
        available_exts
            .names()
            .iter()
            .map(|s| CStr::from_bytes_with_nul(s).unwrap().to_str().unwrap())
            .collect::<Vec<_>>()
    );

    // todo: switch to vulkan
    assert!(available_exts.khr_opengl_es_enable);

    let mut selected_exts = xr::ExtensionSet::default();
    selected_exts.bd_body_tracking = true;
    selected_exts.bd_controller_interaction = true;
    selected_exts.bd_facial_simulation = true;
    selected_exts.ext_eye_gaze_interaction = true;
    selected_exts.ext_hand_tracking = true;
    selected_exts.ext_local_floor = true;
    selected_exts.ext_performance_settings = true;
    selected_exts.ext_user_presence = true;
    selected_exts.fb_body_tracking = true;
    selected_exts.fb_color_space = true;
    selected_exts.fb_composition_layer_settings = true;
    selected_exts.fb_display_refresh_rate = true;
    selected_exts.fb_eye_tracking_social = true;
    selected_exts.fb_face_tracking2 = true;
    selected_exts.fb_foveation = true;
    selected_exts.fb_foveation_configuration = true;
    selected_exts.fb_passthrough = true;
    selected_exts.fb_swapchain_update_state = true;
    selected_exts.htc_facial_tracking = true;
    selected_exts.htc_passthrough = true;
    selected_exts.htc_vive_focus3_controller_interaction = true;
    #[cfg(target_os = "android")]
    {
        selected_exts.khr_android_create_instance = true;
    }
    selected_exts.khr_convert_timespec_time = true;
    selected_exts.khr_opengl_es_enable = true;
    selected_exts.meta_body_tracking_full_body = true;
    selected_exts.meta_simultaneous_hands_and_controllers = true;
    selected_exts.meta_detached_controllers = true;
    selected_exts.other = [
        META_BODY_TRACKING_FIDELITY_EXTENSION_NAME,
        BD_MOTION_TRACKING_EXTENSION_NAME,
        PICO_CONFIGURATION_EXTENSION_NAME,
    ]
    .into_iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();

    if provisional_platform == Platform::ThinkRealityVRX {
        // Qualcomm Spaces advertises extensions from both the runtime and implicit API layers,
        // but its runtime rejects some of the optional Meta/Pico extensions during instance
        // creation. Keep this list aligned with extensions that the VRX Unity diagnostics report
        // as enabled for the same runtime and the graphics/performance extensions used by ALVR.
        selected_exts = vrx_extensions();
    }

    if provisional_platform != Platform::ThinkRealityVRX {
        selected_exts = selected_exts.intersection(&available_exts);
    }

    let available_layers = xr_entry.enumerate_layers().unwrap();
    info!("OpenXR available layers: {available_layers:#?}");

    let app_info = xr::ApplicationInfo {
        application_name: "ALVR Client",
        application_version: 0,
        engine_name: "ALVR",
        engine_version: 0,
        api_version: openxr_version,
    };

    let xr_instance = xr_entry
        .create_instance(&app_info, &selected_exts, &[])
        .unwrap_or_else(|e| {
            if provisional_platform == Platform::ThinkRealityVRX {
                panic!(
                    "Failed to create ThinkReality VRX OpenXR instance with required extensions: {e:?}"
                )
            } else {
                panic!("Failed to create OpenXR instance: {e:?}")
            }
        });

    let other_exts = selected_exts
        .other
        .iter()
        .map(|vec| {
            CStr::from_bytes_with_nul(vec)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();

    let platform = alvr_system_info::platform(
        xr_instance
            .properties()
            .ok()
            .map(|s| s.runtime_name.to_owned()),
        xr_instance
            .properties()
            .ok()
            .map(|s| s.runtime_version.into_raw()),
    );
    info!("Detected platform: {platform}");
    alvr_localization::set_language(if platform == Platform::ThinkRealityVRX {
        Language::Chinese
    } else {
        Language::detect()
    });

    let graphics_context = Rc::new(GraphicsContext::new_gl());

    let mut last_lobby_message = String::new();

    'session_loop: loop {
        let xr_system = xr_instance
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .unwrap();

        // mandatory call
        let _ = xr_instance
            .graphics_requirements::<xr::OpenGlEs>(xr_system)
            .unwrap();

        let (xr_session, mut xr_frame_waiter, mut xr_frame_stream) =
            create_session(&xr_instance, xr_system, &graphics_context);

        if platform == Platform::ThinkRealityVRX {
            configure_vrx_runtime(&xr_instance, &xr_session);
        }

        let views_config = xr_instance
            .enumerate_view_configuration_views(
                xr_system,
                xr::ViewConfigurationType::PRIMARY_STEREO,
            )
            .unwrap();
        assert_eq!(views_config.len(), 2);

        let runtime_recommended_view_resolution = UVec2::new(
            views_config[0].recommended_image_rect_width,
            views_config[0].recommended_image_rect_height,
        );

        let runtime_max_view_resolution = UVec2::new(
            views_config[0].max_image_rect_width,
            views_config[0].max_image_rect_height,
        );
        let default_view_resolution = runtime_recommended_view_resolution;
        let max_view_resolution = runtime_max_view_resolution;
        info!(
            "OpenXR view resolution: recommended {default_view_resolution}, max {max_view_resolution}"
        );

        let refresh_rates = if selected_exts.fb_display_refresh_rate {
            if platform == Platform::ThinkRealityVRX {
                xr_session
                    .enumerate_display_refresh_rates()
                    .unwrap_or_else(|e| {
                        panic!("Failed to enumerate ThinkReality VRX display refresh rates: {e:?}")
                    })
            } else {
                xr_session.enumerate_display_refresh_rates().unwrap()
            }
        } else {
            vec![VRX_MAX_REFRESH_RATE_HZ]
        };
        info!("OpenXR supported display refresh rates: {refresh_rates:?}");

        if selected_exts.fb_color_space {
            xr_session
                .set_color_space(xr::ColorSpaceFB::REC709)
                .unwrap();
        }

        let capabilities = ClientCapabilities {
            platform,
            default_view_resolution,
            max_view_resolution,
            refresh_rates,
            // ALVR's fixed-center encoding reconstruction uses its GLES shader,
            // independently of eye tracking and OpenXR swapchain foveation.
            foveated_encoding: platform != Platform::Unknown,
            encoder_high_profile: platform != Platform::Unknown,
            encoder_10_bits: platform != Platform::Unknown,
            encoder_av1: matches!(
                platform,
                Platform::Quest3 | Platform::Quest3S | Platform::Pico4Ultra
            ),
            prefer_10bit: false,
            preferred_encoding_gamma: 1.0,
            prefer_hdr: false,
        };

        let permissions = {
            #[cfg(not(target_os = "android"))]
            {
                vec![]
            }
            #[cfg(target_os = "android")]
            {
                let mut permissions = vec![];

                // Eye-tracking permission is needed for gaze input in InteractionContext.
                if extra_extensions::supports_eye_gaze_interaction(&xr_session, xr_system) {
                    match platform {
                        Platform::QuestPro => {
                            permissions.push("com.oculus.permission.EYE_TRACKING");
                        }
                        Platform::PicoNeo3 | Platform::Pico4Pro | Platform::Pico4Enterprise => {
                            permissions.push("com.picovr.permission.EYE_TRACKING");
                        }
                        _ => {}
                    }
                }

                permissions
            }
        };

        let core_context = Arc::new(ClientCoreContext::new(capabilities, permissions));

        let interaction_context = Arc::new(RwLock::new(InteractionContext::new(
            xr_session.clone(),
            other_exts.clone(),
            xr_system,
            platform,
        )));

        let mut lobby = Lobby::new(
            xr_session.clone(),
            Rc::clone(&graphics_context),
            Arc::clone(&interaction_context),
            platform,
            UVec2::min(default_view_resolution * 2, max_view_resolution),
            &last_lobby_message,
        );

        // For Meta/Quest enabling body tracking would disable multimodal input
        let lobby_body_tracking_config = if platform.is_pico() {
            Some(BodyTrackingSourcesConfig {
                bd: BodyTrackingBDConfig::BodyTracking {
                    high_accuracy: true,
                    prompt_calibration_on_start: false,
                },
                meta: Default::default(),
            })
        } else {
            None
        };
        let lobby_interaction_sources = InteractionSourcesConfig {
            face_tracking: None,
            body_tracking: lobby_body_tracking_config,
            prefers_multimodal_input: true,
        };
        interaction_context
            .write()
            .select_sources(&lobby_interaction_sources);

        let mut session_running = false;
        let mut stream_context = None::<StreamContext>;
        let mut passthrough_layer = None;

        let mut event_storage = xr::EventDataBuffer::new();
        let mut headset_is_worn = true;
        'render_loop: loop {
            if shutdown.value() {
                drop(stream_context.take());
                core_context.pause();
                break 'session_loop;
            }
            while let Some(event) = xr_instance.poll_event(&mut event_storage).unwrap() {
                match event {
                    xr::Event::EventsLost(event) => {
                        error!("OpenXR: lost {} events!", event.lost_event_count());
                    }
                    xr::Event::InstanceLossPending(_) => break 'session_loop,
                    xr::Event::SessionStateChanged(event) => match event.state() {
                        xr::SessionState::READY => {
                            xr_session
                                .begin(xr::ViewConfigurationType::PRIMARY_STEREO)
                                .unwrap();

                            #[cfg(target_os = "android")]
                            if platform == Platform::ThinkRealityVRX {
                                // This thread performs xrWaitFrame and submits GLES frames.
                                // Spaces only raises priority for RENDERER_WORKER.
                                let settings = xr_instance
                                    .exts()
                                    .khr_android_thread_settings
                                    .expect("VRX requires XR_KHR_android_thread_settings");
                                let result = unsafe {
                                    (settings.set_android_application_thread)(
                                        xr_session.as_raw(),
                                        xr::AndroidThreadTypeKHR::RENDERER_WORKER,
                                        libc::gettid() as u32,
                                    )
                                };
                                assert_eq!(
                                    result,
                                    xr::sys::Result::SUCCESS,
                                    "Qualcomm render-thread registration failed: {result:?}"
                                );
                            }

                            core_context.resume();

                            passthrough_layer = PassthroughLayer::new(&xr_session, platform).ok();

                            session_running = true;
                        }
                        xr::SessionState::STOPPING => {
                            session_running = false;

                            passthrough_layer = None;

                            core_context.pause();

                            if platform == Platform::ThinkRealityVRX {
                                xr_session.end().expect("Failed to end VRX OpenXR session");
                            } else {
                                xr_session.end().ok();
                            }
                        }
                        xr::SessionState::EXITING if platform == Platform::ThinkRealityVRX => {
                            drop(stream_context.take());
                            core_context.pause();
                            break 'session_loop;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                            break 'render_loop;
                        }
                        _ => (),
                    },
                    xr::Event::ReferenceSpaceChangePending(event) => {
                        info!(
                            "ReferenceSpaceChangePending type: {:?}",
                            event.reference_space_type()
                        );

                        lobby.update_reference_space();

                        if let Some(stream) = &mut stream_context {
                            stream.update_reference_space();
                        }
                    }
                    xr::Event::PerfSettingsEXT(event) => {
                        info!(
                            "Perf: from {:?} to {:?}, domain: {:?}/{:?}",
                            event.from_level(),
                            event.to_level(),
                            event.domain(),
                            event.sub_domain(),
                        );
                    }
                    xr::Event::InteractionProfileChanged(_)
                    | xr::Event::PassthroughStateChangedFB(_) => {
                        // todo
                    }
                    xr::Event::UserPresenceChangedEXT(event) => {
                        debug!("user present: {:?}", event.is_user_present());
                        headset_is_worn = event.is_user_present();

                        core_context.send_proximity_state(event.is_user_present());
                    }
                    _ => (),
                }
            }

            if !session_running {
                thread::sleep(Duration::from_millis(100));
                continue;
            }

            while let Some(event) = core_context.poll_event() {
                match event {
                    ClientCoreEvent::UpdateHudMessage(message) => {
                        last_lobby_message.clone_from(&message);
                        lobby.update_hud_message(&message);
                    }
                    ClientCoreEvent::StreamingStarted(config) => {
                        if platform == Platform::ThinkRealityVRX {
                            vrx::validate_stream_settings(&config.settings).unwrap_or_else(|e| {
                                panic!("ThinkReality VRX stream configuration: {e:#}")
                            });
                        }
                        let config = ParsedStreamConfig::new(&config);

                        let context = StreamContext::new(
                            Arc::clone(&core_context),
                            xr_session.clone(),
                            Rc::clone(&graphics_context),
                            Arc::clone(&interaction_context),
                            config,
                        );

                        if !context.uses_passthrough() {
                            passthrough_layer = None;
                        }

                        stream_context = Some(context);

                        core_context.send_proximity_state(headset_is_worn);
                    }
                    ClientCoreEvent::StreamingStopped => {
                        if passthrough_layer.is_none() {
                            passthrough_layer = PassthroughLayer::new(&xr_session, platform).ok();
                        }

                        interaction_context
                            .write()
                            .select_sources(&lobby_interaction_sources);

                        stream_context = None;
                    }
                    ClientCoreEvent::Haptics {
                        device_id,
                        duration,
                        frequency,
                        amplitude,
                    } => {
                        let idx = if device_id == *HAND_LEFT_ID { 0 } else { 1 };
                        let action =
                            &interaction_context.read().hands_interaction[idx].vibration_action;

                        action
                            .apply_feedback(
                                &xr_session,
                                xr::Path::NULL,
                                &xr::HapticVibration::new()
                                    .amplitude(amplitude.clamp(0.0, 1.0))
                                    .frequency(frequency.max(0.0))
                                    .duration(xr::Duration::from_nanos(duration.as_nanos() as _)),
                            )
                            .unwrap();
                    }
                    ClientCoreEvent::DecoderConfig { codec, config_nal } => {
                        if let Some(stream) = &mut stream_context {
                            stream.maybe_initialize_decoder(codec, config_nal);
                        }
                    }
                    ClientCoreEvent::RealTimeConfig(config) => {
                        if platform == Platform::ThinkRealityVRX {
                            vrx::validate_real_time_config(&config).unwrap_or_else(|e| {
                                panic!("ThinkReality VRX real-time configuration: {e:#}")
                            });
                        }
                        if config.passthrough.is_some() && passthrough_layer.is_none() {
                            passthrough_layer = PassthroughLayer::new(&xr_session, platform).ok();
                        } else if config.passthrough.is_none() && passthrough_layer.is_some() {
                            passthrough_layer = None;
                        }

                        if let Some(cpu_performance_level) = &config.cpu_performance_level {
                            set_performance_level(
                                &xr_instance,
                                &xr_session,
                                xr::PerfSettingsDomainEXT::CPU,
                                cpu_performance_level.clone(),
                                platform,
                            );
                        }

                        if let Some(gpu_performance_level) = &config.gpu_performance_level {
                            set_performance_level(
                                &xr_instance,
                                &xr_session,
                                xr::PerfSettingsDomainEXT::GPU,
                                gpu_performance_level.clone(),
                                platform,
                            );
                        }

                        if let Some(stream) = &mut stream_context {
                            stream.update_real_time_config(&config);
                        }
                    }
                }
            }

            let frame_state = match xr_frame_waiter.wait() {
                Ok(state) => state,
                Err(e) => {
                    error!("{e}");
                    panic!();
                }
            };
            let frame_interval =
                Duration::from_nanos(frame_state.predicted_display_period.as_nanos() as _);
            let vsync_time =
                Duration::from_nanos(frame_state.predicted_display_time.as_nanos() as _);

            xr_frame_stream.begin().unwrap();

            if platform == Platform::ThinkRealityVRX
                && let Some(stream) = &stream_context
            {
                stream.notify_vsync();
            }

            if !frame_state.should_render {
                xr_frame_stream
                    .end(
                        frame_state.predicted_display_time,
                        xr::EnvironmentBlendMode::OPAQUE,
                        &[],
                    )
                    .unwrap();

                continue;
            }

            // todo: allow rendering lobby and stream layers at the same time and add cross fade
            let (layer, display_time) = if let Some(stream) = &mut stream_context {
                stream.render(frame_interval, vsync_time)
            } else {
                (lobby.render(vsync_time), vsync_time)
            };

            let layers: &[&xr::CompositionLayerBase<_>] =
                if let Some(passthrough_layer) = &passthrough_layer {
                    &[passthrough_layer, &layer.build()]
                } else {
                    &[&layer.build()]
                };

            graphics_context.make_current();
            let res = xr_frame_stream.end(
                to_xr_time(display_time),
                xr::EnvironmentBlendMode::OPAQUE,
                layers,
            );

            if let Err(e) = res {
                let time = to_xr_time(display_time);
                error!("End frame failed! {e}, timestamp: {display_time:?}, time: {time:?}");

                if platform == Platform::ThinkRealityVRX {
                    panic!("VRX xrEndFrame failed: {e}; no substitute frame is submitted");
                }

                if !platform.is_vive() {
                    xr_frame_stream
                        .end(
                            frame_state.predicted_display_time,
                            xr::EnvironmentBlendMode::OPAQUE,
                            &[],
                        )
                        .unwrap();
                }
            }
        }
    }

    // graphics_context is dropped here, after the session and input threads.
    platform
}

#[allow(unused)]
fn xr_runtime_now(xr_instance: &xr::Instance) -> Option<xr::Time> {
    xr_instance
        .now()
        .ok()
        .filter(|&time_nanos| time_nanos.as_nanos() > 0)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    use android_activity::{InputStatus, MainEvent, PollEvent};
    use jni::refs::Reference;

    let shutdown = Arc::new(RelaxedAtomic::new(false));
    let render_shutdown = Arc::clone(&shutdown);
    let rendering_thread = thread::spawn(move || {
        // workaround for the Pico runtime
        let context = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) };
        vm.attach_current_thread(|env| {
            // Also close the VRX activity after a fatal initialization/config
            // error instead of leaving an activity with a dead render thread.
            // Resume unwinding below so the original failure remains visible.
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&render_shutdown)));
            let platform = match &outcome {
                Ok(platform) => *platform,
                Err(_) => alvr_system_info::platform(None, None),
            };
            if platform == Platform::ThinkRealityVRX && !render_shutdown.value() {
                // EXITING must end the Android experience too. The C entry point
                // leaves activity ownership with its embedding application.
                env.call_method(
                    unsafe {
                        jni::objects::JObject::global_kind_from_raw(context.context().cast())
                    },
                    jni::jni_str!("finishAndRemoveTask"),
                    jni::jni_sig!("()V"),
                    &[],
                )?;
            }

            if let Err(payload) = outcome {
                std::panic::resume_unwind(payload);
            }

            jni::errors::Result::Ok(())
        })
        .unwrap();
    });

    let mut should_quit = false;
    while !should_quit {
        app.poll_events(Some(Duration::from_millis(100)), |event| match event {
            PollEvent::Main(MainEvent::Destroy) => {
                shutdown.set(true);
                should_quit = true;
            }
            PollEvent::Main(MainEvent::InputAvailable) => {
                if let Ok(mut iter) = app.input_events_iter() {
                    while iter.next(|_| InputStatus::Unhandled) {}
                }
            }
            _ => (),
        });
    }

    // Android destruction also signals the render loop when no XR event arrives.
    rendering_thread.join().unwrap();
}

#[cfg(test)]
mod tests {
    use super::vrx_extensions;

    #[test]
    fn vrx_requires_only_validated_extensions() {
        let extensions = vrx_extensions();
        assert!(!extensions.qcom_tracking_optimization_settings);
        assert!(extensions.ext_performance_settings);
        assert!(extensions.fb_display_refresh_rate);
        assert!(extensions.khr_convert_timespec_time);
        assert!(extensions.khr_opengl_es_enable);
        assert!(!extensions.other.is_empty());
    }
}
