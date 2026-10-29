use crate::{
    graphics::{self, ProjectionLayerAlphaConfig, ProjectionLayerBuilder},
    interaction::{self, InteractionContext, InteractionSourcesConfig},
};
use alvr_client_core::{
    ClientCoreContext, VideoFrameMetadata,
    video_decoder::{self, VideoDecoderConfig, VideoDecoderSource},
};
use alvr_common::{
    AlvrFoveatedEncodingParams, DETACHED_CONTROLLER_LEFT_ID, DETACHED_CONTROLLER_RIGHT_ID,
    HAND_LEFT_ID, HAND_RIGHT_ID, HEAD_ID, Pose, RelaxedAtomic, ViewParams,
    anyhow::Result,
    error,
    glam::{UVec2, Vec2},
    info,
    parking_lot::{Condvar, Mutex, RwLock},
};
use alvr_graphics::{GraphicsContext, StreamRenderer, StreamViewParams};
use alvr_packets::{ClientStreamConfig, RealTimeConfig, TrackingData};
use alvr_session::{
    ClientsideFoveationConfig, ClientsideFoveationMode, ClientsidePostProcessingConfig, CodecType,
    MediacodecProperty, PassthroughMode, UpscalingConfig,
};
use alvr_system_info::Platform;
use openxr as xr;
use std::{
    ptr,
    rc::Rc,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const DECODER_MAX_TIMEOUT_MULTIPLIER: f32 = 0.8;

fn decoder_poll_deadline(platform: Platform, frame_interval: Duration) -> Option<Instant> {
    if platform == Platform::ThinkRealityVRX {
        // Do not block inside the OpenXR frame window waiting for decode. If
        // the decoder has no frame ready, reuse the last staged image and keep
        // the low-latency submission phase.
        None
    } else {
        Some(
            Instant::now()
                + Duration::from_secs_f32(
                    frame_interval.as_secs_f32() * DECODER_MAX_TIMEOUT_MULTIPLIER,
                ),
        )
    }
}

fn poll_decoder_frame<T>(
    mut poll: impl FnMut() -> Option<T>,
    deadline: Option<Instant>,
) -> Option<T> {
    let mut frame = poll();

    if let Some(deadline) = deadline {
        while frame.is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_micros(500));
            frame = poll();
        }
    }

    frame
}

fn openxr_display_time(
    platform: Platform,
    video_timestamp: Duration,
    predicted_display_time: Duration,
    predicted_display_period: Duration,
    use_custom_reprojection: bool,
) -> Duration {
    if platform == Platform::ThinkRealityVRX || use_custom_reprojection {
        // OpenXR requires xrEndFrame::displayTime to use the monotonic time
        // returned by xrWaitFrame. VRX rejects a video timestamp that is
        // repeated across display frames as a duplicate frame submission.
        predicted_display_time.saturating_sub(
            predicted_display_period
                * (platform == Platform::ThinkRealityVRX)
                    .then(vrx_display_lead_frames)
                    .unwrap_or(0),
        )
    } else {
        // Keep the established ALVR presentation timestamp on other runtimes.
        Duration::max(
            video_timestamp,
            predicted_display_time.saturating_sub(Duration::from_secs(1)),
        )
    }
}

#[cfg(target_os = "android")]
fn vrx_display_lead_frames() -> u32 {
    use std::{
        ffi::{CStr, CString, c_char},
        sync::LazyLock,
    };

    unsafe extern "C" {
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
    }

    static LEAD_FRAMES: LazyLock<u32> = LazyLock::new(|| {
        let Ok(name) = CString::new("debug.alvr.vrx_display_lead_frames") else {
            return 0;
        };
        let mut value = [0 as c_char; 92];
        let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
        if len <= 0 {
            return 0;
        }

        unsafe { CStr::from_ptr(value.as_ptr()) }
            .to_str()
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
            .min(3)
    });

    *LEAD_FRAMES
}

#[cfg(not(target_os = "android"))]
fn vrx_display_lead_frames() -> u32 {
    0
}

fn input_poll_subdivisions(platform: Platform) -> u32 {
    if platform == Platform::ThinkRealityVRX {
        // The vendor player sends one VSync-aligned pose update per 90 Hz frame.
        // Avoid the generic 3x input polling overhead on the XR2.
        1
    } else {
        3
    }
}

fn needs_limited_range_fix(platform: Platform, enable_hdr: bool) -> bool {
    // Qualcomm's GLES MediaCodec path already samples the decoded surface in
    // the display's expected range. Applying ALVR's full-range compatibility
    // compression again darkens the image and reduces contrast.
    !enable_hdr && platform != Platform::ThinkRealityVRX && platform != Platform::SamsungGalaxyXR
}

#[derive(Default)]
struct VsyncSignal {
    sequence: Mutex<u64>,
    condvar: Condvar,
}

impl VsyncSignal {
    fn sequence(&self) -> u64 {
        *self.sequence.lock()
    }

    fn notify(&self) {
        let mut sequence = self.sequence.lock();
        *sequence = sequence.wrapping_add(1);
        self.condvar.notify_one();
    }

    fn wait_for_next(&self, running: &RelaxedAtomic, observed: &mut u64) -> bool {
        let mut sequence = self.sequence.lock();
        while *sequence == *observed && running.value() {
            self.condvar.wait(&mut sequence);
        }

        // Remember the last consumed frame across input polls, including when
        // the render thread notifies before this thread starts waiting.
        *observed = *sequence;
        running.value()
    }
}

pub struct ParsedStreamConfig {
    pub view_resolution: UVec2,
    pub refresh_rate_hint: f32,
    pub encoding_gamma: f32,
    pub enable_hdr: bool,
    pub passthrough: Option<PassthroughMode>,
    pub foveated_encoding_config: Option<AlvrFoveatedEncodingParams>,
    pub clientside_foveation_config: Option<ClientsideFoveationConfig>,
    pub clientside_post_processing: Option<ClientsidePostProcessingConfig>,
    pub upscaling: Option<UpscalingConfig>,
    pub force_software_decoder: bool,
    pub max_buffering_frames: f32,
    pub buffering_history_weight: f32,
    pub decoder_options: Vec<(String, MediacodecProperty)>,
    pub interaction_sources: InteractionSourcesConfig,
}

impl ParsedStreamConfig {
    pub fn new(config: &ClientStreamConfig) -> Self {
        Self {
            view_resolution: config.negotiated_config.view_resolution,
            refresh_rate_hint: config.negotiated_config.refresh_rate_hint,
            encoding_gamma: config.negotiated_config.encoding_gamma,
            enable_hdr: config.negotiated_config.enable_hdr,
            passthrough: config.settings.video.passthrough.as_option().cloned(),
            foveated_encoding_config: config.negotiated_config.foveated_encoding,
            clientside_foveation_config: config
                .settings
                .video
                .clientside_foveation
                .as_option()
                .cloned(),
            clientside_post_processing: config
                .settings
                .video
                .clientside_post_processing
                .as_option()
                .cloned(),
            upscaling: config.settings.video.upscaling.as_option().cloned(),
            force_software_decoder: config.settings.video.force_software_decoder,
            max_buffering_frames: config.settings.video.max_buffering_frames,
            buffering_history_weight: config.settings.video.buffering_history_weight,
            decoder_options: config.settings.video.mediacodec_extra_options.clone(),
            interaction_sources: InteractionSourcesConfig::new(config),
        }
    }
}

pub struct StreamContext {
    core_context: Arc<ClientCoreContext>,
    xr_session: xr::Session<xr::OpenGlEs>,
    interaction_context: Arc<RwLock<InteractionContext>>,
    stage_reference_space: Arc<xr::Space>,
    view_reference_space: Arc<xr::Space>,
    swapchains: [xr::Swapchain<xr::OpenGlEs>; 2],
    last_good_video_frame_metadata: VideoFrameMetadata,
    input_thread: Option<JoinHandle<()>>,
    input_thread_running: Arc<RelaxedAtomic>,
    config: ParsedStreamConfig,
    target_view_resolution: UVec2,
    renderer: StreamRenderer,
    decoder: Option<(VideoDecoderConfig, VideoDecoderSource)>,
    use_custom_reprojection: bool,
    effective_refresh_rate: f32,
    vsync_signal: Arc<VsyncSignal>,
}

impl StreamContext {
    pub fn new(
        core_ctx: Arc<ClientCoreContext>,
        xr_session: xr::Session<xr::OpenGlEs>,
        gfx_ctx: Rc<GraphicsContext>,
        interaction_ctx: Arc<RwLock<InteractionContext>>,
        config: ParsedStreamConfig,
    ) -> StreamContext {
        interaction_ctx
            .write()
            .select_sources(&config.interaction_sources);

        let xr_exts = xr_session.instance().exts();

        let effective_refresh_rate = if core_ctx.platform() == Platform::ThinkRealityVRX {
            assert!(
                xr_exts.fb_display_refresh_rate.is_some(),
                "ThinkReality VRX must support XR_FB_display_refresh_rate"
            );
            let requested_rate = config.refresh_rate_hint;
            let supported_rates =
                xr_session
                    .enumerate_display_refresh_rates()
                    .unwrap_or_else(|e| {
                        panic!("Failed to enumerate ThinkReality VRX display refresh rates: {e:?}")
                    });
            if !supported_rates.contains(&requested_rate) {
                panic!(
                    "ThinkReality VRX does not support requested refresh rate {requested_rate}Hz; supported: {supported_rates:?}"
                );
            }

            xr_session
                .request_display_refresh_rate(requested_rate)
                .unwrap_or_else(|e| {
                    panic!(
                        "Failed to request ThinkReality VRX refresh rate {requested_rate}Hz: {e:?}"
                    )
                });
            let actual_rate = xr_session.get_display_refresh_rate().unwrap_or_else(|e| {
                panic!("Failed to query ThinkReality VRX display refresh rate: {e:?}")
            });
            if actual_rate != requested_rate {
                info!(
                    "ThinkReality VRX accepted {requested_rate}Hz but reports {actual_rate}Hz; the runtime may apply the request asynchronously"
                );
            }
            actual_rate
        } else {
            if xr_exts.fb_display_refresh_rate.is_some() {
                xr_session
                    .request_display_refresh_rate(config.refresh_rate_hint)
                    .unwrap();
            }
            config.refresh_rate_hint
        };
        info!(
            "Effective display refresh rate: {effective_refresh_rate} Hz (requested {} Hz)",
            config.refresh_rate_hint
        );

        let foveation_profile = if let Some(config) = &config.clientside_foveation_config
            && xr_exts.fb_swapchain_update_state.is_some()
            && xr_exts.fb_foveation.is_some()
            && xr_exts.fb_foveation_configuration.is_some()
        {
            let level;
            let dynamic;
            match config.mode {
                ClientsideFoveationMode::Static { level: lvl } => {
                    level = lvl;
                    dynamic = false;
                }
                ClientsideFoveationMode::Dynamic { max_level } => {
                    level = max_level;
                    dynamic = true;
                }
            };

            xr_session
                .create_foveation_profile(Some(xr::FoveationLevelProfile {
                    level: xr::FoveationLevelFB::from_raw(level as i32),
                    vertical_offset: config.vertical_offset_deg,
                    dynamic: xr::FoveationDynamicFB::from_raw(dynamic as i32),
                }))
                .ok()
        } else {
            None
        };

        let target_view_resolution = alvr_graphics::compute_target_view_resolution(
            config.view_resolution,
            &config.upscaling,
        );
        let format = graphics::swapchain_format(
            &gfx_ctx,
            &xr_session,
            config.enable_hdr,
            core_ctx.platform(),
        );

        let swapchains = [
            graphics::create_swapchain(
                &xr_session,
                &gfx_ctx,
                target_view_resolution,
                format,
                foveation_profile.as_ref(),
            ),
            graphics::create_swapchain(
                &xr_session,
                &gfx_ctx,
                target_view_resolution,
                format,
                foveation_profile.as_ref(),
            ),
        ];

        let renderer = StreamRenderer::new(
            gfx_ctx,
            config.view_resolution,
            target_view_resolution,
            [
                swapchains[0]
                    .enumerate_images()
                    .unwrap()
                    .iter()
                    .map(|i| *i as _)
                    .collect(),
                swapchains[1]
                    .enumerate_images()
                    .unwrap()
                    .iter()
                    .map(|i| *i as _)
                    .collect(),
            ],
            format,
            config.foveated_encoding_config,
            !((core_ctx.platform().is_pico()
                || (core_ctx.platform() == Platform::SamsungGalaxyXR))
                && config.enable_hdr),
            // TODO: Find a driver heuristic for the limited range bug instead?
            needs_limited_range_fix(core_ctx.platform(), config.enable_hdr),
            config.encoding_gamma,
            config.upscaling.clone(),
        );

        {
            let int_ctx = interaction_ctx.read();
            core_ctx.send_active_interaction_profile(
                *HAND_LEFT_ID,
                int_ctx.hands_interaction[0].controllers_profile_id,
                int_ctx.hands_interaction[0].input_ids.clone(),
            );
            core_ctx.send_active_interaction_profile(
                *HAND_RIGHT_ID,
                int_ctx.hands_interaction[1].controllers_profile_id,
                int_ctx.hands_interaction[1].input_ids.clone(),
            );
        }

        let input_thread_running = Arc::new(RelaxedAtomic::new(false));

        let stage_reference_space = Arc::new(interaction::get_reference_space(
            &xr_session,
            xr::ReferenceSpaceType::STAGE,
        ));
        let view_reference_space = Arc::new(interaction::get_reference_space(
            &xr_session,
            xr::ReferenceSpaceType::VIEW,
        ));

        let mut this = StreamContext {
            use_custom_reprojection: core_ctx.platform().is_yvr(),
            core_context: core_ctx,
            xr_session,
            interaction_context: interaction_ctx,
            stage_reference_space,
            view_reference_space,
            swapchains,
            last_good_video_frame_metadata: VideoFrameMetadata {
                view_params: [ViewParams::DUMMY; 2],
                foveation_center_shifts: None,
            },
            input_thread: None,
            input_thread_running,
            config,
            target_view_resolution,
            renderer,
            decoder: None,
            effective_refresh_rate,
            vsync_signal: Arc::new(VsyncSignal::default()),
        };

        this.update_reference_space();

        this
    }

    pub fn uses_passthrough(&self) -> bool {
        self.config.passthrough.is_some()
    }

    pub fn update_reference_space(&mut self) {
        self.input_thread_running.set(false);

        self.stage_reference_space = Arc::new(interaction::get_reference_space(
            &self.xr_session,
            xr::ReferenceSpaceType::STAGE,
        ));
        self.view_reference_space = Arc::new(interaction::get_reference_space(
            &self.xr_session,
            xr::ReferenceSpaceType::VIEW,
        ));

        self.core_context.send_playspace(
            self.xr_session
                .reference_space_bounds_rect(xr::ReferenceSpaceType::STAGE)
                .unwrap()
                .map(|a| Vec2::new(a.width, a.height)),
        );

        if let Some(running) = self.input_thread.take() {
            self.vsync_signal.notify();
            running.join().ok();
        }

        self.input_thread_running.set(true);

        self.input_thread = Some(thread::spawn({
            let core_ctx = Arc::clone(&self.core_context);
            let xr_session = self.xr_session.clone();
            let interaction_ctx = Arc::clone(&self.interaction_context);
            let stage_reference_space = Arc::clone(&self.stage_reference_space);
            let view_reference_space = Arc::clone(&self.view_reference_space);
            let refresh_rate = self.effective_refresh_rate;
            let poll_subdivisions = input_poll_subdivisions(self.core_context.platform());
            let sync_to_vsync = self.core_context.platform() == Platform::ThinkRealityVRX;
            let vsync_signal = Arc::clone(&self.vsync_signal);
            let vsync_sequence = vsync_signal.sequence();
            let running = Arc::clone(&self.input_thread_running);
            move || {
                stream_input_loop(
                    &core_ctx,
                    xr_session,
                    &interaction_ctx,
                    &stage_reference_space,
                    &view_reference_space,
                    refresh_rate,
                    poll_subdivisions,
                    sync_to_vsync,
                    &vsync_signal,
                    vsync_sequence,
                    running,
                )
            }
        }));
    }

    pub fn notify_vsync(&self) {
        self.vsync_signal.notify();
    }

    pub fn maybe_initialize_decoder(&mut self, codec: CodecType, config_nal: Vec<u8>) {
        let new_config = VideoDecoderConfig {
            codec,
            force_software_decoder: self.config.force_software_decoder,
            max_buffering_frames: self.config.max_buffering_frames,
            buffering_history_weight: self.config.buffering_history_weight,
            options: self.config.decoder_options.clone(),
            config_buffer: config_nal,
        };

        let maybe_config = if let Some((config, _)) = &self.decoder {
            (new_config != *config).then_some(new_config)
        } else {
            Some(new_config)
        };

        if let Some(config) = maybe_config {
            let (mut sink, source) = video_decoder::create_decoder(config.clone(), {
                let ctx = Arc::clone(&self.core_context);
                move |maybe_timestamp: Result<Duration>| match maybe_timestamp {
                    Ok(timestamp) => ctx.report_frame_decoded(timestamp),
                    Err(e) => ctx.report_fatal_decoder_error(&e.to_string()),
                }
            });
            self.decoder = Some((config, source));

            self.core_context.set_decoder_input_callback(Box::new(
                move |timestamp, buffer| -> bool { sink.push_nal(timestamp, buffer) },
            ));
        }
    }

    pub fn update_real_time_config(&mut self, config: &RealTimeConfig) {
        self.config.passthrough = config.passthrough.clone();
        self.config.clientside_post_processing = config.clientside_post_processing.clone();
    }

    pub fn render(
        &mut self,
        frame_interval: Duration,
        vsync_time: Duration,
    ) -> (ProjectionLayerBuilder<'_>, Duration) {
        let xr_vsync_time = xr::Time::from_nanos(vsync_time.as_nanos() as _);
        let frame_poll_deadline =
            decoder_poll_deadline(self.core_context.platform(), frame_interval);
        let frame_result = self
            .decoder
            .as_mut()
            .and_then(|(_, source)| poll_decoder_frame(|| source.get_frame(), frame_poll_deadline));

        let (timestamp, frame_metadata, buffer_ptr) =
            if let Some((timestamp, buffer_ptr)) = frame_result {
                if let Some(metadata) = self.core_context.report_compositor_start(timestamp) {
                    self.last_good_video_frame_metadata = VideoFrameMetadata {
                        foveation_center_shifts: metadata
                            .foveation_center_shifts
                            .or(self.last_good_video_frame_metadata.foveation_center_shifts),
                        ..metadata
                    };
                }

                // Keep displaying new images even when their metadata is unavailable.
                (timestamp, self.last_good_video_frame_metadata, buffer_ptr)
            } else {
                (
                    vsync_time,
                    self.last_good_video_frame_metadata,
                    ptr::null_mut(),
                )
            };
        let view_params = frame_metadata.view_params;

        let left_swapchain_idx = self.swapchains[0].acquire_image().unwrap();
        let right_swapchain_idx = self.swapchains[1].acquire_image().unwrap();

        self.swapchains[0]
            .wait_image(xr::Duration::INFINITE)
            .unwrap();
        self.swapchains[1]
            .wait_image(xr::Duration::INFINITE)
            .unwrap();

        let (flags, maybe_views) = self
            .xr_session
            .locate_views(
                xr::ViewConfigurationType::PRIMARY_STEREO,
                xr_vsync_time,
                &self.stage_reference_space,
            )
            .unwrap();

        let current_headset_views = if flags.contains(xr::ViewStateFlags::ORIENTATION_VALID) {
            maybe_views
        } else {
            vec![crate::default_view(), crate::default_view()]
        };

        // The poses and FoVs we received from the PC runtime, which may differ and/or include
        // altered FoVs based on settings and view conversions done for canting.
        let input_view_params = view_params;
        let mut output_view_params = input_view_params;
        let openxr_display_time = openxr_display_time(
            self.core_context.platform(),
            timestamp,
            vsync_time,
            frame_interval,
            self.use_custom_reprojection,
        );

        // (shinyquagsire23) I don't entirely trust runtimes to implement CompositionLayerProjectionView
        // correctly, but if we do trust them, avoid doing rotation ourselves. Otherwise, rerender.
        // Ex: YVR/PFDMR has issues with aspect ratio mismatches and passthrough compositing.
        if self.use_custom_reprojection {
            output_view_params = [
                ViewParams {
                    pose: crate::from_xr_pose(current_headset_views[0].pose),
                    fov: crate::from_xr_fov(current_headset_views[0].fov),
                },
                ViewParams {
                    pose: crate::from_xr_pose(current_headset_views[1].pose),
                    fov: crate::from_xr_fov(current_headset_views[1].fov),
                },
            ];
        }

        self.renderer.render(
            buffer_ptr,
            [
                StreamViewParams {
                    swapchain_index: left_swapchain_idx,
                    input_view_params: input_view_params[0],
                    output_view_params: output_view_params[0],
                },
                StreamViewParams {
                    swapchain_index: right_swapchain_idx,
                    input_view_params: input_view_params[1],
                    output_view_params: output_view_params[1],
                },
            ],
            self.config.passthrough.as_ref(),
            frame_metadata
                .foveation_center_shifts
                .map(|centers| centers.map(Vec2::from_array)),
        );

        self.swapchains[0].release_image().unwrap();
        self.swapchains[1].release_image().unwrap();

        if !buffer_ptr.is_null()
            && let Some(xr_now) = crate::xr_runtime_now(self.xr_session.instance())
        {
            self.core_context.report_submit(
                timestamp,
                vsync_time.saturating_sub(Duration::from_nanos(xr_now.as_nanos() as u64)),
            );
        }

        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di {
                width: self.target_view_resolution.x as _,
                height: self.target_view_resolution.y as _,
            },
        };

        let clientside_post_processing = self
            .xr_session
            .instance()
            .exts()
            .fb_composition_layer_settings
            .and(self.config.clientside_post_processing.clone());

        let layer = ProjectionLayerBuilder::new(
            &self.stage_reference_space,
            [
                xr::CompositionLayerProjectionView::new()
                    .pose(crate::to_xr_pose(output_view_params[0].pose))
                    .fov(crate::to_xr_fov(output_view_params[0].fov))
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&self.swapchains[0])
                            .image_array_index(0)
                            .image_rect(rect),
                    ),
                xr::CompositionLayerProjectionView::new()
                    .pose(crate::to_xr_pose(output_view_params[1].pose))
                    .fov(crate::to_xr_fov(output_view_params[1].fov))
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&self.swapchains[1])
                            .image_array_index(0)
                            .image_rect(rect),
                    ),
            ],
            self.config
                .passthrough
                .clone()
                .map(|mode| ProjectionLayerAlphaConfig {
                    premultiplied: matches!(
                        mode,
                        PassthroughMode::Blend {
                            premultiplied_alpha: true,
                            ..
                        } | PassthroughMode::RgbChromaKey(_)
                            | PassthroughMode::HsvChromaKey(_)
                    ),
                }),
            clientside_post_processing,
        );

        (layer, openxr_display_time)
    }
}

impl Drop for StreamContext {
    fn drop(&mut self) {
        self.input_thread_running.set(false);
        self.vsync_signal.notify();
        self.input_thread.take().unwrap().join().ok();
    }
}

fn stream_input_loop(
    core_ctx: &ClientCoreContext,
    xr_session: xr::Session<xr::OpenGlEs>,
    interaction_ctx: &RwLock<InteractionContext>,
    stage_reference_space: &xr::Space,
    view_reference_space: &xr::Space,
    refresh_rate: f32,
    poll_subdivisions: u32,
    sync_to_vsync: bool,
    vsync_signal: &VsyncSignal,
    mut vsync_sequence: u64,
    running: Arc<RelaxedAtomic>,
) {
    let mut last_controller_poses = [Pose::IDENTITY; 2];
    let mut last_palm_poses = [Pose::IDENTITY; 2];
    let mut last_view_params = [ViewParams::DUMMY; 2];

    let mut deadline = Instant::now();
    let frame_interval = Duration::from_secs_f32(1.0 / refresh_rate);
    #[cfg(target_os = "android")]
    if sync_to_vsync
        && crate::android_debug_property("debug.alvr.tracking_diagnostics").as_deref() == Some("1")
    {
        error!("VRX tracked input loop started");
    }
    while running.value() {
        if sync_to_vsync && !vsync_signal.wait_for_next(&running, &mut vsync_sequence) {
            break;
        }

        let int_ctx_lock = interaction_ctx.read();
        let int_ctx = &*int_ctx_lock;
        // Streaming related inputs are updated here. Make sure every input poll is done in this
        // thread
        if let Err(e) = xr_session.sync_actions(&[(&int_ctx.action_set).into()]) {
            error!("{e}");
            return;
        }

        let Some(now) = crate::xr_runtime_now(xr_session.instance()).map(crate::from_xr_time)
        else {
            error!("Cannot poll tracking: invalid time");
            return;
        };

        let target_time = now + core_ctx.get_total_prediction_offset();

        let Some((head_motion, local_views)) = interaction::get_head_data(
            &xr_session,
            core_ctx.platform(),
            stage_reference_space,
            view_reference_space,
            now,
            target_time,
            &last_view_params,
        ) else {
            continue;
        };

        if let Some(views) = local_views {
            core_ctx.send_view_params(views);
            last_view_params = views;
        }

        let mut device_motions = Vec::with_capacity(3);

        device_motions.push((*HEAD_ID, head_motion));

        let left_hand_data = crate::interaction::get_hand_data(
            &xr_session,
            core_ctx.platform(),
            stage_reference_space,
            now,
            target_time,
            &int_ctx.hands_interaction[0],
            &mut last_controller_poses[0],
            &mut last_palm_poses[0],
        );
        let right_hand_data = crate::interaction::get_hand_data(
            &xr_session,
            core_ctx.platform(),
            stage_reference_space,
            now,
            target_time,
            &int_ctx.hands_interaction[1],
            &mut last_controller_poses[1],
            &mut last_palm_poses[1],
        );

        // Note: When multimodal input is enabled, we are sure that when free hands are used
        // (not holding controllers) the controller data is None.
        if (int_ctx.multimodal_hands_enabled || left_hand_data.skeleton_joints.is_none())
            && let Some(motion) = left_hand_data.grip_motion
        {
            device_motions.push((*HAND_LEFT_ID, motion));
        }
        if (int_ctx.multimodal_hands_enabled || right_hand_data.skeleton_joints.is_none())
            && let Some(motion) = right_hand_data.grip_motion
        {
            device_motions.push((*HAND_RIGHT_ID, motion));
        }

        if int_ctx.multimodal_hands_enabled
            && let Some(detached_controller) = left_hand_data.detached_grip_motion
        {
            device_motions.push((*DETACHED_CONTROLLER_LEFT_ID, detached_controller));
        }
        if int_ctx.multimodal_hands_enabled
            && let Some(detached_controller) = right_hand_data.detached_grip_motion
        {
            device_motions.push((*DETACHED_CONTROLLER_RIGHT_ID, detached_controller));
        }

        let face = interaction::get_face_data(
            &xr_session,
            core_ctx.platform(),
            &int_ctx.face_sources,
            stage_reference_space,
            view_reference_space,
            head_motion.pose.orientation,
            now,
        );

        let body = int_ctx
            .body_source
            .as_ref()
            .and_then(|source| interaction::get_body_skeleton(source, stage_reference_space, now));

        if let Some(source) = &int_ctx.body_source {
            device_motions.append(&mut interaction::get_bd_motion_trackers(source, now));
        }

        drop(int_ctx_lock);

        // Even though the server is already adding the motion-to-photon latency, here we use
        // target_time as the poll_timestamp to compensate for the fact that video frames are sent
        // with the poll timestamp instead of the vsync time. This is to ensure correctness when
        // submitting frames to OpenXR. This won't cause any desync with the server because no time
        // sync step is performed between client and server.
        core_ctx.send_tracking(TrackingData {
            poll_timestamp: target_time,
            device_motions,
            hand_skeletons: [
                left_hand_data.skeleton_joints,
                right_hand_data.skeleton_joints,
            ],
            face,
            body,
        });

        let button_entries =
            interaction::update_buttons(&xr_session, &interaction_ctx.read().button_actions);
        if !button_entries.is_empty() {
            core_ctx.send_buttons(button_entries);
        }

        if !sync_to_vsync {
            deadline += frame_interval / poll_subdivisions;
            thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RelaxedAtomic, VsyncSignal, decoder_poll_deadline, input_poll_subdivisions,
        needs_limited_range_fix, openxr_display_time, poll_decoder_frame,
    };
    use alvr_system_info::Platform;
    use std::{
        sync::{Arc, mpsc},
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn vrx_uses_one_input_poll_per_display_frame() {
        assert_eq!(input_poll_subdivisions(Platform::ThinkRealityVRX), 1);
        assert_eq!(input_poll_subdivisions(Platform::Pico4), 3);
    }

    #[test]
    fn vrx_skips_redundant_limited_range_compression() {
        assert!(!needs_limited_range_fix(Platform::ThinkRealityVRX, false));
        assert!(needs_limited_range_fix(Platform::Quest3, false));
        assert!(!needs_limited_range_fix(Platform::Quest3, true));
    }

    #[test]
    fn vrx_uses_the_runtime_predicted_display_time() {
        let task_video_timestamp = Duration::from_millis(1_000);
        let task_predicted_display_time = Duration::from_millis(1_011);

        assert_eq!(
            openxr_display_time(
                Platform::ThinkRealityVRX,
                task_video_timestamp,
                task_predicted_display_time,
                Duration::from_millis(11),
                false,
            ),
            task_predicted_display_time
        );
        assert_eq!(
            openxr_display_time(
                Platform::Quest3,
                task_video_timestamp,
                task_predicted_display_time,
                Duration::from_millis(11),
                false,
            ),
            task_video_timestamp
        );
    }

    #[test]
    fn vrx_does_not_wait_for_a_decoder_frame_inside_the_openxr_frame() {
        let frame_interval = Duration::from_secs_f32(1.0 / 90.0);

        let deadline = decoder_poll_deadline(Platform::ThinkRealityVRX, frame_interval);
        assert_eq!(deadline, None);

        let mut poll_count = 0;
        assert_eq!(
            poll_decoder_frame(
                || {
                    poll_count += 1;
                    None::<()>
                },
                deadline,
            ),
            None
        );
        assert_eq!(poll_count, 1);

        assert!(
            decoder_poll_deadline(Platform::Quest3, frame_interval)
                .is_some_and(|deadline| deadline > Instant::now())
        );
    }

    #[test]
    fn vsync_signal_wakes_the_input_thread() {
        check_vsync_wake(false);
    }

    #[test]
    fn vsync_notification_before_wait_is_not_lost() {
        check_vsync_wake(true);
    }

    fn check_vsync_wake(notify_before_wait: bool) {
        let signal = Arc::new(VsyncSignal::default());
        let running = Arc::new(RelaxedAtomic::new(true));
        let mut observed = signal.sequence();
        if notify_before_wait {
            // Deterministically exercise a delayed input thread. There will
            // be no later frame notification to rescue a lost wake-up.
            signal.notify();
        }
        let waiter_signal = Arc::clone(&signal);
        let waiter_running = Arc::clone(&running);
        let (sender, receiver) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let woke = waiter_signal.wait_for_next(&waiter_running, &mut observed);
            sender.send(woke).unwrap();
        });

        if !notify_before_wait {
            signal.notify();
        }
        let result = receiver.recv_timeout(Duration::from_secs(2));

        // Unblock and join the worker even if the assertion would fail.
        running.set(false);
        signal.notify();
        waiter.join().unwrap();
        assert_eq!(result, Ok(true));
    }

    #[test]
    fn vsync_wait_stops_without_a_later_wake_up() {
        let signal = VsyncSignal::default();
        let running = RelaxedAtomic::new(false);

        signal.notify();

        let mut observed = signal.sequence();
        assert!(!signal.wait_for_next(&running, &mut observed));
    }
}
