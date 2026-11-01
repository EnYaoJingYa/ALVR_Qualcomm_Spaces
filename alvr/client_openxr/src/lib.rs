mod c_api;
mod display_timing;
mod extra_extensions;
mod graphics;
mod interaction;
mod lobby;
mod passthrough;
mod stream;
mod vrx_counter;
#[cfg(target_os = "android")]
mod waiting;

use crate::stream::ParsedStreamConfig;
use alvr_client_core::{ClientCapabilities, ClientCoreContext, ClientCoreEvent};
use alvr_common::{
    Fov, HAND_LEFT_ID, Pose, debug, error,
    glam::{Quat, UVec2, Vec3},
    info,
    parking_lot::RwLock,
};
use alvr_graphics::GraphicsContext;
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
use std::{
    ffi::CStr,
    fmt::{Display, Formatter},
    path::Path,
    rc::Rc,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use stream::StreamContext;

use alvr_system_info::{
    VRX_ENCODED_EYE_RESOLUTION as VRX_DECODE_SURFACE_RESOLUTION,
    VRX_VISIBLE_EYE_RESOLUTION as VRX_NATIVE_EYE_RESOLUTION,
};
const QUEST3_NATIVE_EYE_RESOLUTION: UVec2 = UVec2::new(2064, 2208);

#[derive(Debug, Clone)]
pub(crate) struct StageFailure {
    pub stage: &'static str,
    pub code: String,
    pub retryable: bool,
    pub fallback: &'static str,
    pub allocated_resources: &'static str,
    pub detail: String,
}

impl StageFailure {
    pub(crate) fn new(
        stage: &'static str,
        code: impl Into<String>,
        retryable: bool,
        fallback: &'static str,
        allocated_resources: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            code: code.into(),
            retryable,
            fallback,
            allocated_resources,
            detail: detail.into(),
        }
    }

    pub(crate) fn from_error(
        stage: &'static str,
        error: impl Display,
        retryable: bool,
        fallback: &'static str,
        allocated_resources: &'static str,
    ) -> Self {
        Self::new(
            stage,
            error.to_string(),
            retryable,
            fallback,
            allocated_resources,
            error.to_string(),
        )
    }

    pub(crate) fn log(&self, platform: Platform) {
        #[cfg(target_os = "android")]
        waiting::set_failure(self.clone());
        let platform = if platform == Platform::ThinkRealityVRX {
            "lenovo_thinkreality_vrx"
        } else {
            "other"
        };
        error!(
            "event=openxr_stage_failed platform={platform} stage={} code={} retryable={} \
            allocated_resources={} fallback={} detail={}",
            self.stage,
            self.code,
            self.retryable,
            self.allocated_resources,
            self.fallback,
            self.detail
        );
    }
}

impl Display for StageFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stage={} code={} retryable={} fallback={} allocated_resources={} detail={}",
            self.stage,
            self.code,
            self.retryable,
            self.fallback,
            self.allocated_resources,
            self.detail
        )
    }
}

fn fault_check(stage: &'static str, resources: &'static str) -> Result<(), StageFailure> {
    #[cfg(all(feature = "vrx-fault-injection", target_os = "android"))]
    {
        let mut value = [0 as libc::c_char; 92];
        unsafe {
            libc::__system_property_get(c"debug.alvr.fail_stage".as_ptr(), value.as_mut_ptr());
        }
        let selected = unsafe { CStr::from_ptr(value.as_ptr()) }.to_string_lossy();
        return fault_selected(&selected, stage, resources);
    }
    #[cfg(not(all(feature = "vrx-fault-injection", target_os = "android")))]
    {
        let _ = (stage, resources);
        Ok(())
    }
}

#[cfg(any(test, all(feature = "vrx-fault-injection", target_os = "android")))]
fn fault_selected(
    selected: &str,
    stage: &'static str,
    resources: &'static str,
) -> Result<(), StageFailure> {
    if selected == stage {
        Err(StageFailure::new(
            stage,
            "injected_failure",
            true,
            "release_resources_and_wait_or_retry_with_backoff",
            resources,
            "Local fault injection",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    #[test]
    fn injected_failures_preserve_stage_code_and_recovery_metadata() {
        for stage in [
            "loader",
            "instance",
            "session",
            "swapchain",
            "input",
            "decoder",
            "private_service",
        ] {
            let error = fault_selected(stage, stage, "test_resource").unwrap_err();
            assert_eq!(error.stage, stage);
            assert_eq!(error.code, "injected_failure");
            assert!(error.retryable);
            assert_eq!(error.allocated_resources, "test_resource");
            assert!(!error.fallback.is_empty());
            assert!(fault_selected("", stage, "none").is_ok());
        }
    }
}

fn vrx_extensions(available: &xr::ExtensionSet) -> xr::ExtensionSet {
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
    extensions.other = [b"XR_QCOM_component_versioning\0".to_vec()]
        .into_iter()
        .collect();
    extensions.intersection(available)
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
) {
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
) -> Result<
    (
        xr::Session<xr::OpenGlEs>,
        xr::FrameWaiter,
        xr::FrameStream<xr::OpenGlEs>,
    ),
    StageFailure,
> {
    fault_check("session", "instance,graphics_context")?;
    #[allow(unreachable_code)]
    unsafe {
        xr_instance
            .create_session(xr_system, &graphics::session_create_info(graphics_context))
            .map_err(|error| {
                StageFailure::from_error(
                    "session_create",
                    error,
                    true,
                    "destroy_session_resources_and_retry_with_backoff",
                    "instance,graphics_context,frame_waiter,frame_stream",
                )
            })
    }
}

pub fn entry_point() {
    alvr_client_core::init_logging();

    const LEGACY_OPENXR_VERSION: xr::Version = xr::Version::new(1, 0, 34);
    const CURRENT_OPENXR_VERSION: xr::Version = xr::Version::new(1, 1, 36);

    // Runtime-independent platform data is available before OpenXR starts.
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
    if let Err(error) = fault_check("loader", "none") {
        error.log(provisional_platform);
        return;
    }
    let xr_entry = match unsafe {
        xr::Entry::load_from(Path::new(&format!("libopenxr_loader{loader_suffix}.so")))
    } {
        Ok(entry) => entry,
        Err(error) => {
            StageFailure::from_error(
                "loader_load",
                error,
                false,
                "show_runtime_unavailable_message_and_return_to_waiting_activity",
                "none",
            )
            .log(provisional_platform);
            return;
        }
    };

    #[cfg(target_os = "android")]
    if let Err(error) = xr_entry.initialize_android_loader() {
        StageFailure::from_error(
            "loader_initialize",
            error,
            true,
            "destroy_loader_and_return_to_waiting_activity",
            "loader",
        )
        .log(provisional_platform);
        return;
    }

    let available_exts = match xr_entry.enumerate_extensions() {
        Ok(extensions) => extensions,
        Err(error) => {
            StageFailure::from_error(
                "extension_query",
                error,
                true,
                "destroy_loader_and_return_to_waiting_activity",
                "loader",
            )
            .log(provisional_platform);
            return;
        }
    };
    info!(
        "OpenXR available extensions: {:#?}",
        available_exts
            .names()
            .iter()
            .filter_map(|s| CStr::from_bytes_with_nul(s).ok())
            .filter_map(|s| s.to_str().ok())
            .collect::<Vec<_>>()
    );

    // todo: switch to vulkan
    if !available_exts.khr_opengl_es_enable {
        StageFailure::new(
            "graphics_requirement",
            "XR_ERROR_EXTENSION_NOT_PRESENT",
            false,
            "public_media_codec_path_unavailable_return_to_waiting_activity",
            "loader",
            "XR_KHR_opengl_es_enable is required",
        )
        .log(provisional_platform);
        return;
    }

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

    selected_exts = if provisional_platform == Platform::ThinkRealityVRX {
        // Spaces advertises optional Meta/Pico extensions that cannot be
        // enabled together. Start with the smallest validated native set.
        vrx_extensions(&available_exts)
    } else {
        selected_exts.intersection(&available_exts)
    };

    let available_layers = match xr_entry.enumerate_layers() {
        Ok(layers) => layers,
        Err(error) => {
            StageFailure::from_error(
                "layer_query",
                error,
                true,
                "continue_without_optional_layers",
                "loader,instance",
            )
            .log(provisional_platform);
            return;
        }
    };
    info!("OpenXR available layers: {available_layers:#?}");

    let other_exts = selected_exts
        .other
        .iter()
        .filter_map(|vec| CStr::from_bytes_with_nul(vec).ok())
        .filter_map(|name| name.to_str().ok())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if let Err(error) = fault_check("instance", "loader") {
        error.log(provisional_platform);
        return;
    }
    let xr_instance = match xr_entry.create_instance(
        &xr::ApplicationInfo {
            application_name: "ALVR Client",
            application_version: 0,
            engine_name: "ALVR",
            engine_version: 0,
            api_version: openxr_version,
        },
        &selected_exts,
        &[],
    ) {
        Ok(instance) => instance,
        Err(error) => {
            StageFailure::from_error(
                "instance_create",
                error,
                true,
                "destroy_loader_and_return_to_waiting_activity",
                "loader",
            )
            .log(provisional_platform);
            return;
        }
    };

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

    info!("event=openxr_stage_start stage=graphics_context_create");
    let graphics_context = match GraphicsContext::try_new_gl() {
        Ok(context) => Rc::new(context),
        Err(error) => {
            StageFailure::from_error(
                "graphics_context_create",
                error,
                true,
                "return_to_waiting_activity",
                "instance",
            )
            .log(platform);
            return;
        }
    };
    info!("event=openxr_stage_complete stage=graphics_context_create");

    let mut last_lobby_message = String::new();
    let mut session_retry_count = 0_u32;

    loop {
        info!("event=openxr_stage_start stage=system_query");
        let xr_system = match xr_instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY) {
            Ok(system) => system,
            Err(error) => {
                StageFailure::from_error(
                    "system_query",
                    error,
                    true,
                    "destroy_instance_and_return_to_waiting_activity",
                    "instance",
                )
                .log(platform);
                break;
            }
        };
        info!("event=openxr_stage_complete stage=system_query");

        info!("event=openxr_stage_start stage=view_configuration_query");
        let views_config = match xr_instance.enumerate_view_configuration_views(
            xr_system,
            xr::ViewConfigurationType::PRIMARY_STEREO,
        ) {
            Ok(views) => views,
            Err(error) => {
                StageFailure::from_error(
                    "view_configuration_query",
                    error,
                    true,
                    "destroy_instance_and_return_to_waiting_activity",
                    "instance,system",
                )
                .log(platform);
                break;
            }
        };
        info!("event=openxr_stage_complete stage=view_configuration_query");
        if views_config.len() != 2 {
            StageFailure::new(
                "view_configuration_validate",
                "invalid_stereo_view_count",
                false,
                "return_to_waiting_activity",
                "instance,system",
                format!("expected 2 views, received {}", views_config.len()),
            )
            .log(platform);
            break;
        }

        let runtime_default_view_resolution = UVec2::new(
            views_config[0].recommended_image_rect_width,
            views_config[0].recommended_image_rect_height,
        );

        let runtime_max_view_resolution = UVec2::new(
            views_config[0].max_image_rect_width,
            views_config[0].max_image_rect_height,
        );

        info!(
            "event=openxr_view_capabilities recommended={runtime_default_view_resolution} \
            maximum={runtime_max_view_resolution}"
        );

        if platform == Platform::ThinkRealityVRX {
            let properties = match xr_instance.system_properties(xr_system) {
                Ok(properties) => properties.graphics_properties,
                Err(error) => {
                    StageFailure::from_error(
                        "system_graphics_properties",
                        error,
                        true,
                        "return_to_waiting_activity",
                        "instance,system",
                    )
                    .log(platform);
                    break;
                }
            };
            let max_swapchain = UVec2::new(
                properties.max_swapchain_image_width,
                properties.max_swapchain_image_height,
            );
            info!("event=openxr_swapchain_capabilities maximum={max_swapchain}");
            if max_swapchain.x < VRX_NATIVE_EYE_RESOLUTION.x
                || max_swapchain.y < VRX_NATIVE_EYE_RESOLUTION.y
            {
                StageFailure::new(
                    "view_configuration_validate",
                    "native_resolution_unsupported",
                    false,
                    "return_to_waiting_activity",
                    "instance,system",
                    format!("VRX requires 2280x2280; maximum swapchain is {max_swapchain}"),
                )
                .log(platform);
                break;
            }
            // Stock Spaces reports 3040x1520 view limits even to Lenovo's RVRPlayer,
            // which requests 2160x2160 through xrCreateSwapchain. For VRX only, try
            // the native size within the system swapchain limit and require actual
            // creation/submission to succeed. Never fall back to a smaller stream.
            if runtime_max_view_resolution.x < VRX_NATIVE_EYE_RESOLUTION.x
                || runtime_max_view_resolution.y < VRX_NATIVE_EYE_RESOLUTION.y
            {
                info!(
                    "event=vrx_native_swapchain_probe requested={VRX_NATIVE_EYE_RESOLUTION} \
                    reported_view_max={runtime_max_view_resolution} system_max={max_swapchain}"
                );
            }
        }

        // The physical VRX LCDs are 2280x2280 per eye. The stock runtime's
        // reported view size is smaller and would visibly reduce text detail.
        let (default_view_resolution, max_view_resolution) = match platform {
            Platform::ThinkRealityVRX => (VRX_NATIVE_EYE_RESOLUTION, VRX_DECODE_SURFACE_RESOLUTION),
            Platform::Quest3 => {
                let native = UVec2::new(
                    QUEST3_NATIVE_EYE_RESOLUTION
                        .x
                        .min(runtime_max_view_resolution.x),
                    QUEST3_NATIVE_EYE_RESOLUTION
                        .y
                        .min(runtime_max_view_resolution.y),
                );
                (native, runtime_max_view_resolution)
            }
            _ => (runtime_default_view_resolution, runtime_max_view_resolution),
        };
        info!(
            "View resolution: runtime recommended {runtime_default_view_resolution}, runtime max \
            {runtime_max_view_resolution}, negotiated default {default_view_resolution}, \
            negotiated max {max_view_resolution}"
        );

        // mandatory call
        info!("event=openxr_stage_start stage=graphics_requirements");
        if let Err(error) = xr_instance.graphics_requirements::<xr::OpenGlEs>(xr_system) {
            StageFailure::from_error(
                "graphics_requirements",
                error,
                false,
                "disable_streaming_and_return_to_waiting_activity",
                "instance,system",
            )
            .log(platform);
            break;
        }
        info!("event=openxr_stage_complete stage=graphics_requirements");

        info!("event=openxr_stage_start stage=session_create");
        let (xr_session, mut xr_frame_waiter, mut xr_frame_stream) =
            match create_session(&xr_instance, xr_system, &graphics_context) {
                Ok(session) => session,
                Err(error) => {
                    error.log(platform);
                    if error.retryable && session_retry_count < 5 {
                        let delay = Duration::from_secs(1 << session_retry_count);
                        session_retry_count += 1;
                        thread::sleep(delay);
                        continue;
                    }
                    break;
                }
            };
        info!("event=openxr_stage_complete stage=session_create");

        let (refresh_rates, refresh_rate_source) = if selected_exts.fb_display_refresh_rate {
            match xr_session.enumerate_display_refresh_rates() {
                Ok(rates) => (rates, "runtime_enumeration"),
                Err(error) => {
                    StageFailure::from_error(
                        "display_refresh_rate_query",
                        error,
                        true,
                        "continue_with_90hz_capability",
                        "instance,system,session",
                    )
                    .log(platform);
                    (vec![90.0], "query_error_fallback")
                }
            }
        } else {
            (vec![90.0], "extension_unavailable_fallback")
        };
        info!(
            "event=display_refresh_capabilities source={refresh_rate_source} rates={refresh_rates:?}"
        );

        if selected_exts.fb_color_space {
            if let Err(error) = xr_session.set_color_space(xr::ColorSpaceFB::REC709) {
                StageFailure::from_error(
                    "color_space_select",
                    error,
                    true,
                    "continue_with_runtime_default_color_space",
                    "instance,system,session",
                )
                .log(platform);
            }
        }

        let capabilities = ClientCapabilities {
            platform,
            default_view_resolution,
            max_view_resolution,
            refresh_rates,
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

        info!("event=openxr_stage_start stage=input_initialize");
        let interaction_context = match InteractionContext::new(
            xr_session.clone(),
            other_exts.clone(),
            xr_system,
            platform,
        ) {
            Ok(context) => Arc::new(RwLock::new(context)),
            Err(error) => {
                error.log(platform);
                if error.retryable && session_retry_count < 5 {
                    let delay = Duration::from_secs(1 << session_retry_count);
                    session_retry_count += 1;
                    drop(core_context);
                    drop(xr_frame_stream);
                    drop(xr_frame_waiter);
                    drop(xr_session);
                    thread::sleep(delay);
                    continue;
                }
                break;
            }
        };
        info!("event=openxr_stage_complete stage=input_initialize");

        info!("event=openxr_stage_start stage=lobby_swapchain_create");
        let mut lobby = match Lobby::new(
            xr_session.clone(),
            Rc::clone(&graphics_context),
            Arc::clone(&interaction_context),
            platform,
            if platform == Platform::ThinkRealityVRX {
                VRX_NATIVE_EYE_RESOLUTION
            } else {
                alvr_system_info::lobby_resolution(
                    runtime_default_view_resolution,
                    runtime_max_view_resolution,
                    2.0,
                )
            },
            &last_lobby_message,
        ) {
            Ok(lobby) => lobby,
            Err(error) => {
                error.log(platform);
                if error.retryable && session_retry_count < 5 {
                    let delay = Duration::from_secs(1 << session_retry_count);
                    session_retry_count += 1;
                    drop(interaction_context);
                    drop(core_context);
                    drop(xr_frame_stream);
                    drop(xr_frame_waiter);
                    drop(xr_session);
                    thread::sleep(delay);
                    continue;
                }
                break;
            }
        };
        info!("event=openxr_stage_complete stage=lobby_swapchain_create");

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
        let mut session_ready_since = None;
        let mut first_frame_submitted = false;
        let mut timing_sample_index = 0u64;
        #[allow(unused_assignments)]
        let mut reconnect_reason = None;
        'render_loop: loop {
            if let Some(detail) = core_context.take_fatal_decoder_error() {
                reconnect_reason = Some(StageFailure::new(
                    "decoder_runtime",
                    "MEDIA_CODEC_RUNTIME_FAILED",
                    true,
                    "destroy_session_and_retry_with_backoff",
                    "instance,session,swapchains,renderer,decoder",
                    detail,
                ));
                break 'render_loop;
            }
            'event_loop: loop {
                let maybe_event = match xr_instance.poll_event(&mut event_storage) {
                    Ok(event) => event,
                    Err(error) => {
                        reconnect_reason = Some(StageFailure::from_error(
                            "event_poll",
                            error,
                            true,
                            "reconnect_session",
                            "instance,session,interaction_context,swapchains,renderer,decoder",
                        ));
                        break 'render_loop;
                    }
                };
                let Some(event) = maybe_event else {
                    break 'event_loop;
                };
                match event {
                    xr::Event::EventsLost(event) => {
                        error!("OpenXR: lost {} events!", event.lost_event_count());
                    }
                    xr::Event::InstanceLossPending(_) => {
                        reconnect_reason = Some(StageFailure::new(
                            "instance_loss",
                            "XR_ERROR_INSTANCE_LOST",
                            false,
                            "destroy_instance_and_return_to_waiting_activity",
                            "instance,session,interaction_context,swapchains,renderer,decoder",
                            "the OpenXR runtime requested instance destruction",
                        ));
                        break 'render_loop;
                    }
                    xr::Event::SessionStateChanged(event) => match event.state() {
                        xr::SessionState::READY => {
                            if let Err(error) =
                                xr_session.begin(xr::ViewConfigurationType::PRIMARY_STEREO)
                            {
                                reconnect_reason = Some(StageFailure::from_error(
                                    "session_begin",
                                    error,
                                    true,
                                    "destroy_session_and_retry_with_backoff",
                                    "instance,session,interaction_context,swapchains,renderer,decoder",
                                ));
                                break 'render_loop;
                            }

                            core_context.resume();

                            passthrough_layer = PassthroughLayer::new(&xr_session, platform).ok();

                            session_running = true;
                            session_ready_since = Some(Instant::now());
                        }
                        xr::SessionState::STOPPING => {
                            session_running = false;
                            session_ready_since = None;

                            passthrough_layer = None;

                            core_context.pause();

                            xr_session.end().ok();
                        }
                        xr::SessionState::EXITING => {
                            reconnect_reason = Some(StageFailure::new(
                                "session_state",
                                "openxr_session_exiting",
                                false,
                                "return_to_waiting_activity",
                                "instance,session,interaction_context,swapchains,renderer,decoder",
                                "the OpenXR session exited",
                            ));
                            break 'render_loop;
                        }
                        xr::SessionState::LOSS_PENDING => {
                            reconnect_reason = Some(StageFailure::new(
                                "session_state",
                                "openxr_session_loss_pending",
                                true,
                                "destroy_session_and_retry_with_backoff",
                                "instance,session,interaction_context,swapchains,renderer,decoder",
                                format!("{:?}", event.state()),
                            ));
                            break 'render_loop;
                        }
                        _ => (),
                    },
                    xr::Event::ReferenceSpaceChangePending(event) => {
                        info!(
                            "ReferenceSpaceChangePending type: {:?}",
                            event.reference_space_type()
                        );

                        if let Err(error) = lobby.update_reference_space() {
                            reconnect_reason = Some(error);
                            break 'render_loop;
                        }

                        if let Some(stream) = &mut stream_context {
                            if let Err(error) = stream.update_reference_space() {
                                reconnect_reason = Some(error);
                                break 'render_loop;
                            }
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
                        let config = ParsedStreamConfig::new(&config);

                        let context = match StreamContext::new(
                            Arc::clone(&core_context),
                            xr_session.clone(),
                            Rc::clone(&graphics_context),
                            Arc::clone(&interaction_context),
                            config,
                        ) {
                            Ok(context) => context,
                            Err(error) => {
                                error.log(platform);
                                reconnect_reason = Some(error);
                                break 'render_loop;
                            }
                        };

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

                        if let Err(error) = action.apply_feedback(
                            &xr_session,
                            xr::Path::NULL,
                            &xr::HapticVibration::new()
                                .amplitude(amplitude.clamp(0.0, 1.0))
                                .frequency(frequency.max(0.0))
                                .duration(xr::Duration::from_nanos(duration.as_nanos() as _)),
                        ) {
                            StageFailure::from_error(
                                "haptic_feedback",
                                error,
                                true,
                                "disable_failed_haptic_pulse_and_continue",
                                "instance,session,interaction_context",
                            )
                            .log(platform);
                        }
                    }
                    ClientCoreEvent::DecoderConfig { codec, config_nal } => {
                        if let Some(stream) = &mut stream_context {
                            if let Err(error) = stream.maybe_initialize_decoder(codec, config_nal) {
                                error.log(platform);
                                reconnect_reason = Some(error);
                                break 'render_loop;
                            }
                        }
                    }
                    ClientCoreEvent::RealTimeConfig(config) => {
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
                            );
                        }

                        if let Some(gpu_performance_level) = &config.gpu_performance_level {
                            set_performance_level(
                                &xr_instance,
                                &xr_session,
                                xr::PerfSettingsDomainEXT::GPU,
                                gpu_performance_level.clone(),
                            );
                        }

                        if let Some(stream) = &mut stream_context {
                            stream.update_real_time_config(&config);
                        }
                    }
                }
            }

            let wait_started = Instant::now();
            let frame_state = match xr_frame_waiter.wait() {
                Ok(state) => state,
                Err(error) => {
                    reconnect_reason = Some(StageFailure::from_error(
                        "frame_wait",
                        error,
                        true,
                        "destroy_session_and_retry_with_backoff",
                        "instance,session,interaction_context,swapchains,renderer,decoder",
                    ));
                    break 'render_loop;
                }
            };
            let frame_interval =
                Duration::from_nanos(frame_state.predicted_display_period.as_nanos() as _);
            let vsync_time =
                Duration::from_nanos(frame_state.predicted_display_time.as_nanos() as _);
            let wait_elapsed = wait_started.elapsed();
            if let Some(stream) = &mut stream_context {
                stream.report_vrx_wait();
            }
            timing_sample_index = timing_sample_index.wrapping_add(1);
            let sample_timing = platform == Platform::ThinkRealityVRX
                && stream_context.is_some()
                && timing_sample_index % 90 == 1;
            let wait_now = sample_timing
                .then(|| xr_runtime_now(&xr_instance))
                .flatten();
            let begin_started = Instant::now();

            if let Err(error) = xr_frame_stream.begin() {
                reconnect_reason = Some(StageFailure::from_error(
                    "frame_begin",
                    error,
                    true,
                    "destroy_session_and_retry_with_backoff",
                    "instance,session,interaction_context,swapchains,renderer,decoder",
                ));
                break 'render_loop;
            }

            let begin_elapsed = begin_started.elapsed();
            if !frame_state.should_render {
                if let Some(stream) = &mut stream_context {
                    stream.discard_unused_decoder_frames();
                }
                if let Err(error) = xr_frame_stream.end(
                    frame_state.predicted_display_time,
                    xr::EnvironmentBlendMode::OPAQUE,
                    &[],
                ) {
                    reconnect_reason = Some(StageFailure::from_error(
                        "frame_end_skip",
                        error,
                        true,
                        "destroy_session_and_retry_with_backoff",
                        "instance,session,interaction_context,swapchains,renderer,decoder",
                    ));
                    break 'render_loop;
                }

                continue;
            }

            // todo: allow rendering lobby and stream layers at the same time and add cross fade
            let render_started = Instant::now();
            let rendered = if let Some(stream) = &mut stream_context {
                stream.render(frame_interval, vsync_time)
            } else {
                lobby.render(vsync_time).map(|layer| (layer, vsync_time))
            };
            let (layer, display_time) = match rendered {
                Ok(rendered) => rendered,
                Err(error) => {
                    reconnect_reason = Some(error);
                    break 'render_loop;
                }
            };

            let render_elapsed = render_started.elapsed();
            let layers: &[&xr::CompositionLayerBase<_>] =
                if let Some(passthrough_layer) = &passthrough_layer {
                    &[passthrough_layer, &layer.build()]
                } else {
                    &[&layer.build()]
                };

            if let Err(error) = graphics_context.try_make_current() {
                reconnect_reason = Some(StageFailure::from_error(
                    "graphics_make_current",
                    error,
                    true,
                    "destroy_session_and_retry_with_backoff",
                    "instance,session,swapchains,decoder",
                ));
                break 'render_loop;
            }
            let submit_now = sample_timing
                .then(|| xr_runtime_now(&xr_instance))
                .flatten();
            let end_started = Instant::now();
            let res = xr_frame_stream.end(
                to_xr_time(display_time),
                xr::EnvironmentBlendMode::OPAQUE,
                layers,
            );
            if res.is_ok()
                && let Some(stream) = &mut stream_context
            {
                stream.confirm_vrx_frame();
            }

            if let (Some(wait_now), Some(submit_now)) = (wait_now, submit_now) {
                // Diagnostic only: preserve runtime timestamps and all latency accounting.
                let predicted = frame_state.predicted_display_time.as_nanos() as i128;
                let submitted = display_time.as_nanos() as i128;
                info!(
                    "event=openxr_timing_sample wait_ms={:.3} begin_ms={:.3} render_ms={:.3} end_ms={:.3} period_ms={:.3} predicted_lead_after_wait_ms={:.3} predicted_lead_at_submit_ms={:.3} submitted_lead_ms={:.3} submitted_minus_predicted_ms={:.3}",
                    wait_elapsed.as_secs_f64() * 1000.,
                    begin_elapsed.as_secs_f64() * 1000.,
                    render_elapsed.as_secs_f64() * 1000.,
                    end_started.elapsed().as_secs_f64() * 1000.,
                    frame_interval.as_secs_f64() * 1000.,
                    (predicted - wait_now.as_nanos() as i128) as f64 / 1e6,
                    (predicted - submit_now.as_nanos() as i128) as f64 / 1e6,
                    (submitted - submit_now.as_nanos() as i128) as f64 / 1e6,
                    (submitted - predicted) as f64 / 1e6,
                );
            }

            if res.is_ok() && !first_frame_submitted {
                info!(
                    "event=openxr_first_frame_submitted streaming={} visible_eye={default_view_resolution}",
                    stream_context.is_some()
                );
                first_frame_submitted = true;
            }

            if let Err(e) = res {
                let time = to_xr_time(display_time);
                error!("End frame failed! {e}, timestamp: {display_time:?}, time: {time:?}");

                if !platform.is_vive() {
                    if let Err(fallback_error) = xr_frame_stream.end(
                        frame_state.predicted_display_time,
                        xr::EnvironmentBlendMode::OPAQUE,
                        &[],
                    ) {
                        StageFailure::from_error(
                            "frame_end_fallback",
                            fallback_error,
                            true,
                            "destroy_session_and_retry_with_backoff",
                            "instance,session,interaction_context,swapchains,renderer,decoder",
                        )
                        .log(platform);
                        reconnect_reason = Some(StageFailure::new(
                            "frame_end",
                            "xr_error",
                            true,
                            "destroy_session_and_retry_with_backoff",
                            "instance,session,interaction_context,swapchains,renderer,decoder",
                            e.to_string(),
                        ));
                        break 'render_loop;
                    }
                }
            }
        }

        if let Some(error) = reconnect_reason {
            error.log(platform);
            if session_ready_since.is_some_and(|ready| ready.elapsed() >= Duration::from_secs(30)) {
                session_retry_count = 0;
            }
            if !error.retryable || session_retry_count >= 5 {
                break;
            }
            drop(stream_context);
            drop(passthrough_layer);
            drop(lobby);
            drop(interaction_context);
            drop(core_context);
            drop(xr_frame_stream);
            drop(xr_frame_waiter);
            drop(xr_session);
            let delay = Duration::from_secs(1 << session_retry_count);
            session_retry_count += 1;
            info!(
                "event=openxr_session_reconnect retry={} delay_secs={} previous_stage={}",
                session_retry_count,
                delay.as_secs(),
                error.stage
            );
            thread::sleep(delay);
        }
    }

    // grapics_context is dropped here
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
    use android_activity::input::{InputEvent, KeyAction, MotionAction};
    use android_activity::{InputStatus, MainEvent, PollEvent};

    let start_renderer = || {
        thread::spawn(|| {
            waiting::clear_failure();
            let context = ndk_context::android_context();
            let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) };
            if let Err(error) = vm.attach_current_thread(|_env| {
                entry_point();
                jni::errors::Result::Ok(())
            }) {
                StageFailure::from_error("jni_attach", error, true, "wait_for_user_retry", "none")
                    .log(Platform::ThinkRealityVRX);
            }
        })
    };
    let mut rendering_thread = Some(start_renderer());
    let mut waiting_for_retry = false;
    let mut next_retry = Instant::now();
    let mut should_quit = false;
    while !should_quit {
        if rendering_thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
        {
            if let Some(thread) = rendering_thread.take() {
                if thread.join().is_err() {
                    StageFailure::new(
                        "render_thread",
                        "unexpected_panic",
                        false,
                        "wait_for_user_retry",
                        "released_thread_resources",
                        "Unexpected rendering failure",
                    )
                    .log(Platform::ThinkRealityVRX);
                }
            }
            waiting_for_retry = true;
            next_retry = Instant::now() + Duration::from_secs(2);
            waiting::draw(&app);
        }
        app.poll_events(Some(Duration::from_millis(100)), |event| match event {
            PollEvent::Main(MainEvent::Destroy) => should_quit = true,
            PollEvent::Main(MainEvent::InitWindow { .. } | MainEvent::RedrawNeeded { .. })
                if waiting_for_retry =>
            {
                waiting::draw(&app)
            }
            PollEvent::Main(MainEvent::InputAvailable) => {
                let mut retry = false;
                if let Ok(mut iter) = app.input_events_iter() {
                    while iter.next(|event| {
                        retry |= match event {
                            InputEvent::KeyEvent(key) => {
                                key.action() == KeyAction::Down && key.repeat_count() == 0
                            }
                            InputEvent::MotionEvent(motion) => matches!(
                                motion.action(),
                                MotionAction::Down | MotionAction::ButtonPress
                            ),
                            _ => false,
                        };
                        if waiting_for_retry {
                            InputStatus::Handled
                        } else {
                            InputStatus::Unhandled
                        }
                    }) {}
                }
                if retry && waiting_for_retry && Instant::now() >= next_retry {
                    waiting_for_retry = false;
                    rendering_thread = Some(start_renderer());
                }
            }
            _ => (),
        });
    }
    if let Some(thread) = rendering_thread {
        let _ = thread.join();
    }
}
