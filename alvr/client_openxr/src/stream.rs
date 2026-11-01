use crate::{
    StageFailure,
    display_timing::select_display_time,
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
    parking_lot::RwLock,
};
use alvr_graphics::{GraphicsContext, StreamRenderer, StreamViewParams};
use alvr_packets::{ClientStreamConfig, RealTimeConfig, TrackingData};
use alvr_session::{
    ClientsideFoveationConfig, ClientsideFoveationMode, ClientsidePostProcessingConfig, CodecType,
    FrameSize, MediacodecProperty, PassthroughMode, UpscalingConfig, VrxGraphicsBackend,
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
use alvr_system_info::{VRX_ENCODED_EYE_RESOLUTION, VRX_VISIBLE_EYE_RESOLUTION};

fn vrx_native_padding(surface: UVec2, render: &FrameSize) -> bool {
    let render = match render {
        FrameSize::Scale(scale) => (VRX_VISIBLE_EYE_RESOLUTION.as_vec2() * *scale)
            .round()
            .as_uvec2(),
        FrameSize::Absolute { width, height } => UVec2::new(*width, height.unwrap_or(*width)),
    };
    surface == VRX_ENCODED_EYE_RESOLUTION && render == VRX_VISIBLE_EYE_RESOLUTION
}

pub struct ParsedStreamConfig {
    pub view_resolution: UVec2,
    pub vrx_native_padding: bool,
    pub refresh_rate_hint: f32,
    pub encoding_gamma: f32,
    pub enable_hdr: bool,
    pub passthrough: Option<PassthroughMode>,
    pub foveated_encoding_config: Option<AlvrFoveatedEncodingParams>,
    pub clientside_foveation_config: Option<ClientsideFoveationConfig>,
    pub clientside_post_processing: Option<ClientsidePostProcessingConfig>,
    pub upscaling: Option<UpscalingConfig>,
    pub force_software_decoder: bool,
    pub vrx_graphics_backend: VrxGraphicsBackend,
    pub max_buffering_frames: f32,
    pub buffering_history_weight: f32,
    pub decoder_options: Vec<(String, MediacodecProperty)>,
    pub interaction_sources: InteractionSourcesConfig,
}

impl ParsedStreamConfig {
    fn encoded_view_resolution(&self) -> UVec2 {
        encoded_view_resolution(self.view_resolution, self.foveated_encoding_config.as_ref())
    }

    pub fn new(config: &ClientStreamConfig) -> Self {
        Self {
            view_resolution: config.negotiated_config.view_resolution,
            vrx_native_padding: vrx_native_padding(
                config.negotiated_config.view_resolution,
                &config.settings.video.emulated_headset_view_resolution,
            ),
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
            vrx_graphics_backend: config.settings.video.vrx_graphics_backend,
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
}

impl StreamContext {
    pub fn new(
        core_ctx: Arc<ClientCoreContext>,
        xr_session: xr::Session<xr::OpenGlEs>,
        gfx_ctx: Rc<GraphicsContext>,
        interaction_ctx: Arc<RwLock<InteractionContext>>,
        config: ParsedStreamConfig,
    ) -> Result<StreamContext, StageFailure> {
        interaction_ctx
            .write()
            .select_sources(&config.interaction_sources);

        alvr_common::info!(
            "Stream geometry: encoded={}x{} visible={}x{} refresh={}Hz",
            config.encoded_view_resolution().x,
            config.encoded_view_resolution().y,
            if core_ctx.platform() == Platform::ThinkRealityVRX {
                VRX_VISIBLE_EYE_RESOLUTION.x
            } else {
                config.view_resolution.x
            },
            if core_ctx.platform() == Platform::ThinkRealityVRX {
                VRX_VISIBLE_EYE_RESOLUTION.y
            } else {
                config.view_resolution.y
            },
            config.refresh_rate_hint,
        );

        if core_ctx.platform() == Platform::ThinkRealityVRX
            && config.vrx_graphics_backend == VrxGraphicsBackend::QxrPrivate
        {
            if let Err(error) = crate::fault_check("private_service", "instance,session") {
                error.log(core_ctx.platform());
            }
            alvr_common::warn!(
                "event=vrx_graphics_backend stage=qxr_private_probe code=not_available \
                fallback=public_mediacodec"
            );
        }

        let xr_exts = xr_session.instance().exts();

        if xr_exts.fb_display_refresh_rate.is_some() {
            if let Err(error) = xr_session.request_display_refresh_rate(config.refresh_rate_hint) {
                let actual = xr_session
                    .get_display_refresh_rate()
                    .map_err(|query_error| {
                        StageFailure::from_error(
                            "display_refresh_rate_query",
                            query_error,
                            false,
                            "select_supported_refresh_rate_and_reconnect",
                            "instance,session",
                        )
                    })?;
                validate_refresh_rate(config.refresh_rate_hint, actual)?;
                StageFailure::from_error(
                    "display_refresh_rate",
                    error,
                    false,
                    "continue_at_runtime_default_refresh_rate",
                    "instance,session,interaction_context",
                )
                .log(core_ctx.platform());
            }
        }

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

        let target_view_resolution = if core_ctx.platform() == Platform::ThinkRealityVRX {
            VRX_VISIBLE_EYE_RESOLUTION
        } else {
            alvr_graphics::compute_target_view_resolution(config.view_resolution, &config.upscaling)
        };
        let source_crop_uv =
            if core_ctx.platform() == Platform::ThinkRealityVRX && config.vrx_native_padding {
                Vec2::splat(
                    (VRX_ENCODED_EYE_RESOLUTION.x - VRX_VISIBLE_EYE_RESOLUTION.x) as f32
                        / (2.0 * VRX_ENCODED_EYE_RESOLUTION.x as f32),
                )
            } else {
                Vec2::ZERO
            };
        let format = graphics::swapchain_format(&gfx_ctx, &xr_session, config.enable_hdr).map_err(
            |error| {
                StageFailure::from_error(
                    "stream_swapchain_format",
                    error,
                    true,
                    "fallback_to_public_sdr_format",
                    "instance,session,interaction_context",
                )
            },
        )?;

        let left_swapchain = graphics::create_swapchain(
            &xr_session,
            &gfx_ctx,
            target_view_resolution,
            format,
            foveation_profile.as_ref(),
        )
        .map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_create",
                error,
                true,
                "destroy_swapchains_and_retry_with_backoff",
                "instance,session,interaction_context",
            )
        })?;
        let right_swapchain = graphics::create_swapchain(
            &xr_session,
            &gfx_ctx,
            target_view_resolution,
            format,
            foveation_profile.as_ref(),
        )
        .map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_create",
                error,
                true,
                "destroy_swapchains_and_retry_with_backoff",
                "instance,session,interaction_context,left_swapchain",
            )
        })?;
        let swapchains = [left_swapchain, right_swapchain];

        let left_images = swapchains[0].enumerate_images().map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_images",
                error,
                true,
                "destroy_swapchains_and_retry_with_backoff",
                "instance,session,interaction_context,swapchains",
            )
        })?;
        let right_images = swapchains[1].enumerate_images().map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_images",
                error,
                true,
                "destroy_swapchains_and_retry_with_backoff",
                "instance,session,interaction_context,swapchains",
            )
        })?;

        let renderer = StreamRenderer::new(
            gfx_ctx,
            config.view_resolution,
            target_view_resolution,
            [
                left_images.iter().map(|i| *i as _).collect(),
                right_images.iter().map(|i| *i as _).collect(),
            ],
            format,
            config.foveated_encoding_config,
            source_crop_uv,
            !((core_ctx.platform().is_pico()
                || (core_ctx.platform() == Platform::SamsungGalaxyXR))
                && config.enable_hdr),
            // TODO: Find a driver heuristic for the limited range bug instead?
            core_ctx.platform() != Platform::SamsungGalaxyXR && !config.enable_hdr,
            config.encoding_gamma,
            config.upscaling.clone(),
        )
        .map_err(|error| {
            StageFailure::from_error(
                "stream_renderer_create",
                error,
                true,
                "reconnect_session",
                "instance,session,swapchains",
            )
        })?;

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

        let stage_reference_space = Arc::new(
            interaction::get_reference_space(&xr_session, xr::ReferenceSpaceType::STAGE).map_err(
                |error| {
                    StageFailure::from_error(
                        "stream_stage_reference_space",
                        error,
                        true,
                        "reconnect_session",
                        "instance,session,interaction_context,swapchains,renderer",
                    )
                },
            )?,
        );
        let view_reference_space = Arc::new(
            interaction::get_reference_space(&xr_session, xr::ReferenceSpaceType::VIEW).map_err(
                |error| {
                    StageFailure::from_error(
                        "stream_view_reference_space",
                        error,
                        true,
                        "reconnect_session",
                        "instance,session,interaction_context,swapchains,renderer",
                    )
                },
            )?,
        );

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
        };

        this.update_reference_space()?;

        Ok(this)
    }

    pub fn uses_passthrough(&self) -> bool {
        self.config.passthrough.is_some()
    }

    pub fn update_reference_space(&mut self) -> Result<(), StageFailure> {
        self.input_thread_running.set(false);

        self.stage_reference_space = Arc::new(
            interaction::get_reference_space(&self.xr_session, xr::ReferenceSpaceType::STAGE)
                .map_err(|error| {
                    StageFailure::from_error(
                        "stream_stage_reference_space_update",
                        error,
                        true,
                        "reconnect_session",
                        "instance,session,interaction_context,swapchains,renderer",
                    )
                })?,
        );
        self.view_reference_space = Arc::new(
            interaction::get_reference_space(&self.xr_session, xr::ReferenceSpaceType::VIEW)
                .map_err(|error| {
                    StageFailure::from_error(
                        "stream_view_reference_space_update",
                        error,
                        true,
                        "reconnect_session",
                        "instance,session,interaction_context,swapchains,renderer",
                    )
                })?,
        );

        self.core_context.send_playspace(
            self.xr_session
                .reference_space_bounds_rect(xr::ReferenceSpaceType::STAGE)
                .unwrap_or(None)
                .map(|a| Vec2::new(a.width, a.height)),
        );

        if let Some(running) = self.input_thread.take() {
            running.join().ok();
        }

        self.input_thread_running.set(true);

        self.input_thread = Some(thread::spawn({
            let core_ctx = Arc::clone(&self.core_context);
            let xr_session = self.xr_session.clone();
            let interaction_ctx = Arc::clone(&self.interaction_context);
            let stage_reference_space = Arc::clone(&self.stage_reference_space);
            let view_reference_space = Arc::clone(&self.view_reference_space);
            let refresh_rate = self.config.refresh_rate_hint;
            let running = Arc::clone(&self.input_thread_running);
            move || {
                stream_input_loop(
                    &core_ctx,
                    xr_session,
                    &interaction_ctx,
                    &stage_reference_space,
                    &view_reference_space,
                    refresh_rate,
                    running,
                )
            }
        }));

        Ok(())
    }

    pub fn maybe_initialize_decoder(
        &mut self,
        codec: CodecType,
        config_nal: Vec<u8>,
    ) -> Result<(), StageFailure> {
        crate::fault_check("decoder", "instance,session,swapchains,input_thread")?;
        let new_config = VideoDecoderConfig {
            encoded_resolution: (self.core_context.platform() == Platform::ThinkRealityVRX)
                .then_some(UVec2::new(
                    self.config.encoded_view_resolution().x * 2,
                    self.config.encoded_view_resolution().y,
                )),
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
            // Release the previous decoder and its worker before allocating its replacement.
            self.decoder.take();
            let (mut sink, source) = video_decoder::create_decoder(config.clone(), {
                let ctx = Arc::clone(&self.core_context);
                move |maybe_timestamp: Result<Duration>| match maybe_timestamp {
                    Ok(timestamp) => ctx.report_frame_decoded(timestamp),
                    Err(e) => ctx.report_fatal_decoder_error(&e.to_string()),
                }
            })
            .map_err(|error| {
                StageFailure::new(
                    "decoder_create",
                    "media_codec_error",
                    true,
                    "destroy_decoder_and_retry_public_media_codec",
                    "instance,session,interaction_context,swapchains,renderer",
                    format!("{error:#}"),
                )
            })?;
            self.decoder = Some((config, source));

            self.core_context.set_decoder_input_callback(Box::new(
                move |timestamp, buffer| -> bool { sink.push_nal(timestamp, buffer) },
            ));
        }

        Ok(())
    }

    pub fn discard_unused_decoder_frames(&mut self) {
        if let Some((_, decoder)) = &mut self.decoder {
            decoder.discard_unused_frames();
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
    ) -> Result<(ProjectionLayerBuilder<'_>, Duration), StageFailure> {
        let xr_vsync_time = xr::Time::from_nanos(vsync_time.as_nanos() as _);
        let frame_poll_deadline = Instant::now()
            + Duration::from_secs_f32(
                frame_interval.as_secs_f32() * DECODER_MAX_TIMEOUT_MULTIPLIER,
            );
        let mut frame_result = None;
        if let Some((_, source)) = &mut self.decoder {
            while frame_result.is_none() && Instant::now() < frame_poll_deadline {
                frame_result = source.get_frame();
                thread::sleep(Duration::from_micros(500));
            }
        }

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

        let left_swapchain_idx = self.swapchains[0].acquire_image().map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_acquire",
                error,
                true,
                "reconnect_session",
                "instance,session,interaction_context,swapchains,renderer,decoder",
            )
        })?;
        let right_swapchain_idx = self.swapchains[1].acquire_image().map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_acquire",
                error,
                true,
                "reconnect_session",
                "instance,session,interaction_context,swapchains,renderer,decoder",
            )
        })?;

        self.swapchains[0]
            .wait_image(xr::Duration::INFINITE)
            .map_err(|error| {
                StageFailure::from_error(
                    "stream_swapchain_wait",
                    error,
                    true,
                    "reconnect_session",
                    "instance,session,interaction_context,swapchains,renderer,decoder",
                )
            })?;
        self.swapchains[1]
            .wait_image(xr::Duration::INFINITE)
            .map_err(|error| {
                StageFailure::from_error(
                    "stream_swapchain_wait",
                    error,
                    true,
                    "reconnect_session",
                    "instance,session,interaction_context,swapchains,renderer,decoder",
                )
            })?;

        let (flags, maybe_views) = self
            .xr_session
            .locate_views(
                xr::ViewConfigurationType::PRIMARY_STEREO,
                xr_vsync_time,
                &self.stage_reference_space,
            )
            .map_err(|error| {
                StageFailure::from_error(
                    "stream_locate_views",
                    error,
                    true,
                    "reconnect_session",
                    "instance,session,interaction_context,swapchains,renderer,decoder",
                )
            })?;

        let current_headset_views = if flags.contains(xr::ViewStateFlags::ORIENTATION_VALID) {
            maybe_views
        } else {
            vec![crate::default_view(), crate::default_view()]
        };

        // The poses and FoVs we received from the PC runtime, which may differ and/or include
        // altered FoVs based on settings and view conversions done for canting.
        let input_view_params = view_params;
        let mut output_view_params = input_view_params;
        // Keep the source image's pose and FoV for reprojection, but schedule VRX
        // against this Runtime frame's display deadline, including repeated video.
        let openxr_display_time = select_display_time(
            self.core_context.platform() == Platform::ThinkRealityVRX,
            timestamp,
            vsync_time,
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

        self.renderer
            .render(
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
            )
            .map_err(|error| {
                StageFailure::from_error(
                    "stream_renderer_render",
                    error,
                    true,
                    "reconnect_session",
                    "instance,session,swapchains,decoder",
                )
            })?;

        self.swapchains[0].release_image().map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_release",
                error,
                true,
                "reconnect_session",
                "instance,session,interaction_context,swapchains,renderer,decoder",
            )
        })?;
        self.swapchains[1].release_image().map_err(|error| {
            StageFailure::from_error(
                "stream_swapchain_release",
                error,
                true,
                "reconnect_session",
                "instance,session,interaction_context,swapchains,renderer,decoder",
            )
        })?;

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

        Ok((layer, openxr_display_time))
    }
}

impl Drop for StreamContext {
    fn drop(&mut self) {
        self.input_thread_running.set(false);
        if let Some(input_thread) = self.input_thread.take() {
            input_thread.join().ok();
        }
    }
}

fn stream_input_loop(
    core_ctx: &ClientCoreContext,
    xr_session: xr::Session<xr::OpenGlEs>,
    interaction_ctx: &RwLock<InteractionContext>,
    stage_reference_space: &xr::Space,
    view_reference_space: &xr::Space,
    refresh_rate: f32,
    running: Arc<RelaxedAtomic>,
) {
    let mut last_controller_poses = [Pose::IDENTITY; 2];
    let mut last_palm_poses = [Pose::IDENTITY; 2];
    let mut last_view_params = [ViewParams::DUMMY; 2];
    let mut controller_pose_relation_logged = [false; 2];

    let mut deadline = Instant::now();
    let frame_interval = Duration::from_secs_f32(1.0 / refresh_rate);
    while running.value() {
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

        // Measure the runtime's grip-to-aim transform rather than assuming the
        // coordinate system of Lenovo's original PC streaming driver. Only two
        // samples are logged per stream; tracking packets and poses are unchanged.
        if core_ctx.platform() == Platform::ThinkRealityVRX {
            for (index, hand) in int_ctx.hands_interaction.iter().enumerate() {
                if !controller_pose_relation_logged[index]
                    && hand
                        .grip_action
                        .is_active(&xr_session, xr::Path::NULL)
                        .unwrap_or(false)
                    && hand
                        .aim_action
                        .is_active(&xr_session, xr::Path::NULL)
                        .unwrap_or(false)
                    && let Ok(location) = hand
                        .aim_space
                        .locate(&hand.grip_space, crate::to_xr_time(now))
                    && location.location_flags.contains(
                        xr::SpaceLocationFlags::ORIENTATION_VALID
                            | xr::SpaceLocationFlags::POSITION_VALID,
                    )
                {
                    let p = location.pose.position;
                    let q = location.pose.orientation;
                    alvr_common::info!(
                        "event=vrx_controller_pose_relation hand={} translation_m=[{:.6},{:.6},{:.6}] orientation_xyzw=[{:.6},{:.6},{:.6},{:.6}]",
                        if index == 0 { "left" } else { "right" },
                        p.x,
                        p.y,
                        p.z,
                        q.x,
                        q.y,
                        q.z,
                        q.w,
                    );
                    controller_pose_relation_logged[index] = true;
                }
            }
        }

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

        deadline += frame_interval / 3;
        thread::sleep(deadline.saturating_duration_since(Instant::now()));
    }
}

fn validate_refresh_rate(requested: f32, actual: f32) -> Result<(), StageFailure> {
    if actual.is_finite() && (requested - actual).abs() < 0.1 {
        return Ok(());
    }
    Err(StageFailure::new(
        "display_refresh_rate",
        "refresh_rate_mismatch",
        false,
        "select_runtime_refresh_rate_and_reconnect",
        "instance,session",
        format!(
            "Negotiated {requested} Hz but runtime is {actual} Hz; refusing inconsistent stream timing"
        ),
    ))
}
fn encoded_view_resolution(
    view_resolution: UVec2,
    foveation: Option<&AlvrFoveatedEncodingParams>,
) -> UVec2 {
    foveation.map_or(view_resolution, |params| {
        UVec2::from_array(params.encoded_view_resolution)
    })
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    #[test]
    fn custom_vrx_resolutions_do_not_inherit_native_crop() {
        let native = FrameSize::Absolute {
            width: 2280,
            height: Some(2280),
        };
        assert!(vrx_native_padding(UVec2::splat(2304), &native));
        assert!(vrx_native_padding(
            UVec2::splat(2304),
            &FrameSize::Scale(1.0)
        ));
        assert!(!vrx_native_padding(UVec2::splat(1920), &native));
        assert!(!vrx_native_padding(
            UVec2::splat(2304),
            &FrameSize::Absolute {
                width: 1920,
                height: None
            }
        ));
        assert!(!vrx_native_padding(
            UVec2::splat(2304),
            &FrameSize::Scale(0.8)
        ));
    }

    #[test]
    fn decoder_uses_compressed_dimensions_without_changing_eye_target() {
        let full = VRX_ENCODED_EYE_RESOLUTION;
        assert_eq!(encoded_view_resolution(full, None), UVec2::splat(2304));
        let foveation = AlvrFoveatedEncodingParams {
            encoded_view_resolution: [1440, 1440],
            ..Default::default()
        };
        let encoded = encoded_view_resolution(full, Some(&foveation));
        assert_eq!(UVec2::new(encoded.x * 2, encoded.y), UVec2::new(2880, 1440));
        assert_eq!(VRX_VISIBLE_EYE_RESOLUTION, UVec2::splat(2280));
    }
    #[test]
    fn rejected_request_requires_matching_runtime_rate() {
        assert!(validate_refresh_rate(90.0, 90.0).is_ok());
        let failure = validate_refresh_rate(72.0, 90.0).unwrap_err();
        assert_eq!(failure.code, "refresh_rate_mismatch");
        assert!(!failure.retryable);
        assert!(validate_refresh_rate(90.0, f32::NAN).is_err());
    }
}
