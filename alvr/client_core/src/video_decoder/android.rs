use super::VideoDecoderConfig;
use alvr_common::{
    RelaxedAtomic, ToAny,
    anyhow::{Context, Result, anyhow, bail},
    error, info,
    parking_lot::Mutex,
    warn,
};
use alvr_session::{CodecType, MediacodecPropType};
use ndk::{
    hardware_buffer::HardwareBufferUsage,
    media::{
        image_reader::{AcquireResult, Image, ImageFormat, ImageReader},
        media_codec::{
            DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec,
            MediaCodecDirection, MediaFormat,
        },
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::c_void,
    ops::Deref,
    ptr,
    sync::{Arc, Weak, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct FakeThreadSafe<T>(T);
unsafe impl<T> Send for FakeThreadSafe<T> {}
unsafe impl<T> Sync for FakeThreadSafe<T> {}

impl<T> Deref for FakeThreadSafe<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

static DECODER_WORKER_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
struct DecoderWorkerGuard;
impl Drop for DecoderWorkerGuard {
    fn drop(&mut self) {
        DECODER_WORKER_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
    }
}

type SharedMediaCodec = Arc<FakeThreadSafe<MediaCodec>>;

type FrameTimings = Arc<Mutex<BTreeMap<Duration, (Instant, Option<Instant>)>>>;
// Bound the vendor queue while tolerating transient backpressure within one frame.
const VRX_MAX_DECODER_INPUTS: usize = 3;
const VRX_INPUT_WAIT_BUDGET: Duration = Duration::from_millis(8);

#[cfg(feature = "vrx-fault-injection")]
fn injected_failure(stage: &str) -> bool {
    unsafe extern "C" {
        fn __system_property_get(
            name: *const std::ffi::c_char,
            value: *mut std::ffi::c_char,
        ) -> i32;
    }
    let mut value = [0 as std::ffi::c_char; 92];
    unsafe {
        __system_property_get(c"debug.alvr.fail_stage".as_ptr(), value.as_mut_ptr());
        std::ffi::CStr::from_ptr(value.as_ptr()).to_bytes() == stage.as_bytes()
    }
}

pub struct VideoDecoderSink {
    immediate_output: bool,
    codec_timestamps: Arc<Mutex<super::CodecFrameMap>>,
    timings: FrameTimings,
    inner: Arc<Mutex<Option<SharedMediaCodec>>>,
}

unsafe impl Send for VideoDecoderSink {}

impl VideoDecoderSink {
    // Block until the buffer has been written or timeout is reached. Returns false if timeout.
    pub fn push_frame_nal(&mut self, timestamp: Duration, data: &[u8]) -> Result<bool> {
        let Some(decoder) = &*self.inner.lock() else {
            // This might happen only during destruction
            return Ok(false);
        };

        let deadline = Instant::now() + VRX_INPUT_WAIT_BUDGET;
        if self.immediate_output {
            while self.codec_timestamps.lock().pending() >= VRX_MAX_DECODER_INPUTS {
                if Instant::now() >= deadline {
                    warn!("event=decoder_admission_timeout budget_ms=8 action=wait_for_idr");
                    return Ok(false);
                }
                // Do not hold the timestamp lock: the output worker retires pending inputs.
                thread::sleep(Duration::from_micros(250));
            }
        }
        let input_timeout = if self.immediate_output {
            deadline.saturating_duration_since(Instant::now())
        } else {
            Duration::ZERO
        };
        match decoder.dequeue_input_buffer(input_timeout) {
            Ok(DequeuedInputBufferResult::Buffer(mut buffer)) => {
                if data.len() > buffer.buffer_mut().len() {
                    // Return the acquired slot before reporting the invalid input.
                    decoder.queue_input_buffer(buffer, 0, 0, 0, 0)?;
                    bail!(
                        "stage=decoder_input code=INPUT_TOO_LARGE retryable=true fallback=reconnect size={}",
                        data.len()
                    );
                }
                unsafe {
                    ptr::copy_nonoverlapping(
                        data.as_ptr(),
                        buffer.buffer_mut().as_mut_ptr().cast(),
                        data.len(),
                    )
                };

                let codec_time = if self.immediate_output {
                    self.codec_timestamps.lock().insert(timestamp)
                } else {
                    let mut timings = self.timings.lock();
                    if timings.len() >= 256 {
                        timings.clear();
                    }
                    timings.insert(timestamp, (Instant::now(), None));
                    // Preserve the legacy timestamp convention on other platforms.
                    timestamp.as_nanos() as i64
                };
                if let Err(error) =
                    decoder.queue_input_buffer(buffer, 0, data.len(), codec_time as _, 0)
                {
                    self.codec_timestamps.lock().frames.remove(&codec_time);
                    self.timings.lock().remove(&timestamp);
                    return Err(error.into());
                }

                Ok(true)
            }
            Ok(DequeuedInputBufferResult::TryAgainLater) => {
                if self.immediate_output {
                    warn!("event=decoder_input_timeout action=wait_for_idr");
                }
                Ok(false)
            }
            Err(e) => bail!("{e}"),
        }
    }
}

struct QueuedImage {
    timestamp: Duration,
    image: Image,
    in_use: bool,
}
unsafe impl Send for QueuedImage {}

// Access the image queue synchronously.
pub struct VideoDecoderSource {
    running: Arc<RelaxedAtomic>,
    dequeue_thread: Option<JoinHandle<()>>,
    image_queue: Arc<Mutex<VecDeque<QueuedImage>>>,
    config: VideoDecoderConfig,
    buffering_running_average: f32,
}

unsafe impl Send for VideoDecoderSource {}

impl VideoDecoderSource {
    pub fn discard_unused_frames(&mut self) {
        self.image_queue.lock().retain(|image| image.in_use);
        self.buffering_running_average = 0.;
    }

    // The application MUST finish using the returned buffer before calling this function again
    pub fn dequeue_frame(&mut self) -> Option<(Duration, *mut c_void)> {
        let mut image_queue_lock = self.image_queue.lock();

        if let Some(queued_image) = image_queue_lock.front()
            && queued_image.in_use
        {
            // image is released and ready to be reused by the decoder
            image_queue_lock.pop_front();
        }

        if self.config.encoded_resolution.is_some() {
            // The previous in-use image has been released above. These are already
            // decoded images, so skipping stale ones cannot break codec references.
            // Keep the image and its exact tracking timestamp together.
            while image_queue_lock.len() > 1 {
                image_queue_lock.pop_front();
            }
            self.buffering_running_average = 0.;
        } else {
            self.buffering_running_average = self.buffering_running_average
                * self.config.buffering_history_weight
                + image_queue_lock.len() as f32 * (1. - self.config.buffering_history_weight);
            if self.buffering_running_average > self.config.max_buffering_frames as f32 {
                image_queue_lock.pop_front();
            }
        }

        if let Some(queued_image) = image_queue_lock.front_mut() {
            queued_image.in_use = true;

            match queued_image.image.hardware_buffer() {
                Ok(hardware_buffer) => {
                    Some((queued_image.timestamp, hardware_buffer.as_ptr().cast()))
                }
                Err(error) => {
                    error!(
                        "event=decoder_stage_failed stage=decoder_get_hardware_buffer \
                        fallback=drop_frame detail={error}"
                    );
                    queued_image.in_use = false;
                    image_queue_lock.pop_front();
                    None
                }
            }
        } else {
            // TODO: add back when implementing proper phase sync
            //warn!("Video frame queue underflow!");
            None
        }
    }
}

impl Drop for VideoDecoderSource {
    fn drop(&mut self) {
        self.running.set(false);

        // Destruction of decoder, buffered images and ImageReader
        self.dequeue_thread.take().map(|t| t.join());
    }
}

fn mime_for_codec(codec: CodecType) -> &'static str {
    match codec {
        CodecType::H264 => "video/avc",
        CodecType::Hevc => "video/hevc",
        CodecType::AV1 => "video/av01",
    }
}

// Attempts to create a MediaCodec, and then configure and start it.
fn decoder_setup(
    codec_type: CodecType,
    is_software: bool,
    format: &MediaFormat,
    image_reader: &ImageReader,
) -> Result<MediaCodec> {
    let decoder = if is_software {
        let sw_codec_name = match codec_type {
            CodecType::H264 => "OMX.google.h264.decoder",
            CodecType::Hevc => "OMX.google.hevc.decoder",
            CodecType::AV1 => bail!("AV1 is not supported for software decoding"),
        };
        MediaCodec::from_codec_name(&sw_codec_name)
            .ok_or(anyhow!("no such codec: {}", &sw_codec_name))?
    } else {
        let mime = mime_for_codec(codec_type);
        MediaCodec::from_decoder_type(&mime)
            .ok_or(anyhow!("unable to find decoder for mime type: {}", &mime))?
    };
    decoder
        .configure(
            &format,
            Some(&image_reader.window()?),
            MediaCodecDirection::Decoder,
        )
        .with_context(|| format!("failed to configure decoder"))?;

    decoder
        .start()
        .with_context(|| format!("failed to start decoder"))?;

    Ok(decoder)
}

// Since we leak the ImageReader, and we pass frame_result_callback to it which contains a reference
// to ClientCoreContext, to avoid circular references we need to use a Weak reference.
fn decoder_lifecycle(
    config: VideoDecoderConfig,
    csd_0: Vec<u8>,
    frame_result_callback: Weak<impl Fn(Result<Duration>) + Send + Sync + 'static>,
    running: Arc<RelaxedAtomic>,
    decoder_sink: Arc<Mutex<Option<SharedMediaCodec>>>,
    decoder_ready: mpsc::Sender<Result<()>>,
    image_queue: Arc<Mutex<VecDeque<QueuedImage>>>,
    image_reader: &mut ImageReader,
    timings: FrameTimings,
    codec_timestamps: Arc<Mutex<super::CodecFrameMap>>,
) -> Result<()> {
    #[cfg(feature = "vrx-fault-injection")]
    if injected_failure("decoder_worker") {
        bail!("stage=decoder_start code=INJECTED_INIT_FAILURE retryable=true fallback=reconnect");
    }
    // 2x: keep the target buffering in the middle of the max amount of queuable frames
    let available_buffering_frames = (2. * config.max_buffering_frames).ceil() as usize;

    image_reader.set_image_listener(Box::new({
        let image_queue = Arc::clone(&image_queue);
        let timings = Arc::clone(&timings);
        let mut sample_count = 0u64;
        let immediate_output = config.encoded_resolution.is_some();
        let codec_timestamps = Arc::clone(&codec_timestamps);
        move |image_reader| {
            let mut image_queue_lock = image_queue.lock();

            if image_queue_lock.len() > available_buffering_frames {
                warn!("Video frame queue overflow!");
                if let Some(index) = image_queue_lock.iter().position(|image| !image.in_use) {
                    image_queue_lock.remove(index);
                }
            }

            match image_reader.acquire_next_image() {
                Ok(AcquireResult::Image(image)) => {
                    let timestamp_ns = match image.timestamp() {
                        Ok(timestamp) => timestamp,
                        Err(error) => {
                            error!(
                                "event=decoder_stage_failed stage=decoder_image_timestamp \
                                fallback=drop_frame detail={error}"
                            );
                            return;
                        }
                    };
                    let frame_timing = if immediate_output {
                        codec_timestamps.lock().frames.remove(&(timestamp_ns / 1000)).map(|frame| (frame.timestamp, Some((frame.input, frame.output))))
                    } else {
                        let timestamp = Duration::from_nanos(timestamp_ns as u64);
                        Some((timestamp, timings.lock().remove(&timestamp)))
                    };
                    let Some((timestamp, timing)) = frame_timing else {
                        warn!("event=decoder_timestamp_unmatched image_ns={timestamp_ns}");
                        return;
                    };
                    sample_count += 1;
                    if let Some((input, output)) = timing {
                        if sample_count % 90 == 1 {
                            info!("event=decoder_latency_sample input_to_image_ms={:.3} input_to_output_ms={:.3} output_to_image_ms={:.3}",
                                input.elapsed().as_secs_f64() * 1000.,
                                output.map_or(-1., |t| t.duration_since(input).as_secs_f64() * 1000.),
                                output.map_or(-1., |t| t.elapsed().as_secs_f64() * 1000.));
                        }
                    }

                    if let Some(callback) = frame_result_callback.upgrade() {
                        callback(Ok(timestamp));
                    }

                    image_queue_lock.push_back(QueuedImage {
                        timestamp,
                        image,
                        in_use: false,
                    });
                }
                Ok(e) => {
                    error!("ImageReader error: {e:?}");

                    image_queue_lock.retain(|image| image.in_use);
                }
                Err(e) => {
                    error!("ImageReader error: {e}");

                    image_queue_lock.retain(|image| image.in_use);
                }
            }
        }
    }))?;

    // Documentation says that this call is necessary to properly dispose acquired buffers.
    // todo: find out how to use it and avoid leaking the ImageReader
    image_reader.set_buffer_removed_listener(Box::new(|_, _| ()))?;

    let mime = mime_for_codec(config.codec);

    let mut format = MediaFormat::new();
    format.set_str("mime", mime);
    // Given https://github.com/alvr-org/ALVR/pull/1933#discussion_r1431902906 - change at own risk.
    // It might be harmless, it might not be, but it's definitely a risk.
    let resolution = config
        .encoded_resolution
        .unwrap_or(alvr_common::glam::UVec2::new(512, 1024));
    format.set_i32("width", resolution.x as i32);
    format.set_i32("height", resolution.y as i32);
    format.set_buffer("csd-0", &csd_0);

    for (key, prop) in &config.options {
        let maybe_error = match prop.ty {
            MediacodecPropType::Float => prop
                .value
                .parse()
                .map(|value| format.set_f32(key, value))
                .to_any(),
            MediacodecPropType::Int32 => prop
                .value
                .parse()
                .map(|value| format.set_i32(key, value))
                .to_any(),
            MediacodecPropType::Int64 => prop
                .value
                .parse()
                .map(|value| format.set_i64(key, value))
                .to_any(),
            MediacodecPropType::String => Ok(format.set_str(key, &prop.value)),
        };

        if let Err(e) = maybe_error {
            error!("Failed to set property {key} to {}: {e}", prop.value);
        }
    }

    if config.encoded_resolution.is_some() {
        format.set_i32("low-latency", 1);
    }
    info!("Using AMediaCodec format:{} ", format);

    let decoder = if config.force_software_decoder {
        decoder_setup(config.codec, true, &format, &image_reader)?
    } else {
        // Hardware decoders sometimes fail at the CSD-0.
        // May as well fall back if this occurs.
        match decoder_setup(config.codec, false, &format, &image_reader) {
            Ok(d) => d,
            Err(e) => {
                // would be "warn!" but this is a severe caveat and a pretty major error.
                warn!("event=decoder_hardware_setup_failed detail={e:#}");

                if config.encoded_resolution.is_some() {
                    format.set_i32("low-latency", 0);
                    warn!(
                        "event=decoder_optional_feature_disabled feature=low-latency detail={e:#}"
                    );
                    decoder_setup(config.codec, false, &format, &image_reader)?
                } else {
                    decoder_setup(config.codec, true, &format, &image_reader)?
                }
            }
        }
    };

    info!(
        "event=decoder_configured name={}",
        decoder.name().unwrap_or_else(|_| "unavailable".into())
    );
    let decoder = Arc::new(FakeThreadSafe(decoder));

    {
        let mut decoder_lock = decoder_sink.lock();

        *decoder_lock = Some(Arc::clone(&decoder));

        if !running.value() || decoder_ready.send(Ok(())).is_err() {
            decoder_lock.take();
            decoder.stop()?;
            bail!("stage=decoder_start code=CANCELLED retryable=true fallback=reconnect");
        }
    }

    let mut backlog_watchdog = super::BacklogWatchdog::default();
    let watchdog_epoch = Instant::now();
    let mut error_counter = 0;
    let mut output_count = 0u64;
    let mut last_output = Instant::now();
    let runtime_result = (|| -> Result<()> {
        while running.value() {
            #[cfg(feature = "vrx-fault-injection")]
            if output_count >= 5 && injected_failure("decoder_output") {
                bail!(
                    "stage=decoder_output code=INJECTED_RUNTIME_FAILURE retryable=true fallback=reconnect"
                );
            }
            if config.encoded_resolution.is_some()
                && last_output.elapsed() >= Duration::from_secs(2)
                && codec_timestamps.lock().pending() != 0
            {
                bail!(
                    "stage=decoder_output code=OUTPUT_STALLED retryable=true fallback=reconnect_with_backoff"
                );
            }
            match decoder.dequeue_output_buffer(Duration::from_millis(1)) {
                Ok(DequeuedOutputBufferInfoResult::Buffer(buffer)) => {
                    let codec_time = buffer.info().presentation_time_us();
                    last_output = Instant::now();
                    let input_age = if config.encoded_resolution.is_some() {
                        codec_timestamps
                            .lock()
                            .frames
                            .get_mut(&codec_time)
                            .map(|frame| {
                                frame.output = Some(Instant::now());
                                frame.input.elapsed()
                            })
                    } else {
                        timings
                            .lock()
                            .get_mut(&Duration::from_nanos(codec_time as u64))
                            .map(|frame| {
                                frame.1 = Some(Instant::now());
                                frame.0.elapsed()
                            })
                    };
                    output_count += 1;
                    if output_count % 90 == 1 {
                        info!(
                            "event=decoder_output_sample frame={} codec_pts_us={}",
                            output_count,
                            buffer.info().presentation_time_us()
                        );
                    }
                    // Only the legacy path stores nanoseconds in the codec PTS field.
                    let presentation_time_ns = buffer.info().presentation_time_us();

                    let release_result = if config.encoded_resolution.is_some() {
                        decoder.release_output_buffer(buffer, true)
                    } else {
                        decoder.release_output_buffer_at_time(buffer, presentation_time_ns)
                    };
                    if let Err(e) = release_result {
                        bail!(
                            "stage=decoder_release code=OUTPUT_RELEASE_FAILED retryable=true fallback=reconnect detail={e}"
                        );
                    }
                    if config.encoded_resolution.is_some()
                        && input_age.is_some_and(|age| {
                            backlog_watchdog.expired(watchdog_epoch.elapsed(), age)
                        })
                    {
                        bail!(
                            "stage=decoder_backlog code=SUSTAINED_BACKLOG retryable=true fallback=reconnect_with_backoff"
                        );
                    }
                }
                Ok(DequeuedOutputBufferInfoResult::TryAgainLater) => continue,
                Ok(i) => info!("Decoder dequeue event: {i:?}"),
                Err(e) => {
                    error!("Decoder dequeue error: {e}");

                    error_counter += 1;
                    if error_counter > 10 {
                        bail!("Too many decoder errors: {e}");
                    }

                    // lessen logcat flood (just in case)
                    thread::sleep(Duration::from_millis(50));

                    continue;
                }
            }

            error_counter = 0;
        }

        Ok(())
    })();
    // Release MediaCodec even when the output loop reports an error.
    decoder_sink.lock().take(); // Make sure the shared ref is deleted first
    let stop_result = decoder.stop();
    drop(decoder);
    runtime_result?;
    stop_result?;
    Ok(())
}

// Create a sink/source pair
pub fn video_decoder_split(
    config: VideoDecoderConfig,
    csd_0: Vec<u8>,
    frame_result_callback: impl Fn(Result<Duration>) + Send + Sync + 'static,
) -> Result<(VideoDecoderSink, VideoDecoderSource)> {
    if DECODER_WORKER_ACTIVE
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        bail!(
            "stage=decoder_start code=PREVIOUS_WORKER_ACTIVE retryable=false fallback=wait_for_cleanup"
        );
    }
    let worker_guard = DecoderWorkerGuard;
    let running = Arc::new(RelaxedAtomic::new(true));
    let decoder_sink = Arc::new(Mutex::new(None::<SharedMediaCodec>));
    let (decoder_ready, startup_result) = mpsc::channel();
    let timings = FrameTimings::default();
    let codec_timestamps = Arc::new(Mutex::new(super::CodecFrameMap::default()));
    let image_queue = Arc::new(Mutex::new(VecDeque::<QueuedImage>::new()));

    let dequeue_thread = thread::spawn({
        let config = config.clone();
        let running = Arc::clone(&running);
        let decoder_sink = Arc::clone(&decoder_sink);
        let decoder_ready = decoder_ready.clone();
        let codec_timestamps = Arc::clone(&codec_timestamps);
        let timings = Arc::clone(&timings);
        let image_queue = Arc::clone(&image_queue);
        move || {
            let _worker_guard = worker_guard;
            const MAX_BUFFERING_FRAMES: usize = 10;
            let mut image_reader = match ImageReader::new_with_usage(
                1,
                1,
                ImageFormat::PRIVATE,
                HardwareBufferUsage::GPU_SAMPLED_IMAGE,
                MAX_BUFFERING_FRAMES as i32,
            ) {
                Ok(reader) => reader,
                Err(e) => {
                    let detail = format!(
                        "stage=image_reader code=INIT_FAILED retryable=true fallback=reconnect detail={e}"
                    );
                    let _ = decoder_ready.send(Err(anyhow!(detail.clone())));
                    frame_result_callback(Err(anyhow!(detail)));
                    return;
                }
            };

            let frame_result_callback = Arc::new(frame_result_callback);

            if let Err(e) = decoder_lifecycle(
                config,
                csd_0,
                Arc::downgrade(&frame_result_callback),
                Arc::clone(&running),
                Arc::clone(&decoder_sink),
                decoder_ready.clone(),
                Arc::clone(&image_queue),
                &mut image_reader,
                timings,
                codec_timestamps,
            ) {
                decoder_sink.lock().take();
                let detail = format!(
                    "stage=decoder_start code=INIT_OR_RUNTIME_FAILED retryable=true fallback=reconnect detail={e:#}"
                );
                let _ = decoder_ready.send(Err(anyhow!(detail.clone())));
                frame_result_callback(Err(anyhow!(detail)));
            }

            image_queue.lock().clear();
        }
    });

    drop(decoder_ready);
    match startup_result.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => (),
        result => {
            running.set(false);
            // Completed failure paths are joined. A vendor call that never returns cannot be
            // forcibly terminated safely; cancellation prevents it publishing a late decoder.
            if dequeue_thread.is_finished() {
                let _ = dequeue_thread.join();
            }
            return Err(match result {
                Ok(Err(error)) => error,
                _ => anyhow!(
                    "stage=decoder_start code=STARTUP_TIMEOUT retryable=true fallback=reconnect"
                ),
            });
        }
    }

    let sink = VideoDecoderSink {
        immediate_output: config.encoded_resolution.is_some(),
        codec_timestamps,
        timings,
        inner: decoder_sink,
    };
    let source = VideoDecoderSource {
        running,
        dequeue_thread: Some(dequeue_thread),
        image_queue,
        config,
        buffering_running_average: 0.0,
    };

    Ok((sink, source))
}
