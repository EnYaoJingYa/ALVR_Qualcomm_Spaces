#[cfg(target_os = "android")]
mod android;

use alvr_common::anyhow::Result;
use alvr_session::{CodecType, MediacodecProperty};
use std::time::Duration;

#[cfg(any(target_os = "android", test))]
#[derive(Default)]
struct CodecFrameMap {
    last_time_us: i64,
    frames: std::collections::BTreeMap<i64, CodecFrame>,
}

#[cfg(any(target_os = "android", test))]
struct CodecFrame {
    timestamp: Duration,
    input: std::time::Instant,
    output: Option<std::time::Instant>,
}

#[cfg(any(target_os = "android", test))]
impl CodecFrameMap {
    fn insert(&mut self, timestamp: Duration) -> i64 {
        // Tracking predictions can repeat or move backwards. Codec PTS must still identify
        // each submitted frame uniquely, without losing the original metadata timestamp.
        let time_us = i64::try_from(timestamp.as_micros())
            .unwrap_or(i64::MAX - 1)
            .max(self.last_time_us.saturating_add(1));
        self.last_time_us = time_us;
        if self.frames.len() >= 256 {
            self.frames.retain(|_, frame| frame.output.is_none());
        }
        self.frames.insert(
            time_us,
            CodecFrame {
                timestamp,
                input: std::time::Instant::now(),
                output: None,
            },
        );
        time_us
    }

    fn pending(&self) -> usize {
        self.frames
            .values()
            .filter(|frame| frame.output.is_none())
            .count()
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct VideoDecoderConfig {
    pub codec: CodecType,
    /// Present only for the VRX public decoder path; other platforms retain legacy setup.
    pub encoded_resolution: Option<alvr_common::glam::UVec2>,
    pub force_software_decoder: bool,
    pub max_buffering_frames: f32,
    pub buffering_history_weight: f32,
    pub options: Vec<(String, MediacodecProperty)>,
    pub config_buffer: Vec<u8>,
}

pub struct VideoDecoderSink {
    #[cfg(target_os = "android")]
    inner: android::VideoDecoderSink,
}

impl VideoDecoderSink {
    // returns true if frame has been successfully enqueued
    #[allow(unused_variables)]
    pub fn push_nal(&mut self, timestamp: Duration, nal: &[u8]) -> bool {
        #[cfg(target_os = "android")]
        {
            alvr_common::show_err(self.inner.push_frame_nal(timestamp, nal)).unwrap_or(false)
        }
        #[cfg(not(target_os = "android"))]
        false
    }
}

pub struct VideoDecoderSource {
    identity: alvr_packets::vrx_phase::ImageIdentity,
    #[cfg(target_os = "android")]
    inner: android::VideoDecoderSource,
}

impl VideoDecoderSource {
    /// Release unused images when the runtime temporarily stops rendering.
    pub fn discard_unused_frames(&mut self) {
        #[cfg(target_os = "android")]
        self.inner.discard_unused_frames();
    }

    /// If a frame is available, return the timestamp and the AHardwareBuffer.
    pub fn get_frame(&mut self) -> Option<(Duration, *mut std::ffi::c_void)> {
        self.get_frame_with_identity().map(|(_, timestamp, buffer)| (timestamp, buffer))
    }

    /// Every successfully acquired image is fresh, even with a repeated tracking timestamp.
    pub fn get_frame_with_identity(&mut self) -> Option<(alvr_packets::vrx_phase::ImageIdentity, Duration, *mut std::ffi::c_void)> {
        #[cfg(target_os = "android")]
        {
            let (timestamp, buffer) = self.inner.dequeue_frame()?;
            self.identity.image += 1;
            Some((self.identity, timestamp, buffer))
        }
        #[cfg(not(target_os = "android"))]
        None
    }
}

// report_frame_decoded: (target_timestamp: Duration) -> ()
#[allow(unused_variables)]
pub fn create_decoder(
    config: VideoDecoderConfig,
    report_frame_decoded: impl Fn(Result<Duration>) + Send + Sync + 'static,
) -> Result<(VideoDecoderSink, VideoDecoderSource)> {
    static NEXT_DECODER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let identity = alvr_packets::vrx_phase::ImageIdentity {
        decoder: NEXT_DECODER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        image: 0,
    };
    #[cfg(target_os = "android")]
    {
        let (sink, source) = android::video_decoder_split(
            config.clone(),
            config.config_buffer,
            report_frame_decoded,
        )?;

        Ok((
            VideoDecoderSink { inner: sink },
            VideoDecoderSource { inner: source, identity },
        ))
    }
    #[cfg(not(target_os = "android"))]
    Ok((VideoDecoderSink {}, VideoDecoderSource { identity }))
}

// Recover sustained slow outputs, without treating abandoned image callbacks as input backlog.
#[cfg(any(target_os = "android", test))]
#[derive(Default)]
struct BacklogWatchdog {
    since: Option<Duration>,
}
#[cfg(any(target_os = "android", test))]
impl BacklogWatchdog {
    fn expired(&mut self, now: Duration, output_latency: Duration) -> bool {
        if output_latency <= Duration::from_millis(150) {
            self.since = None;
            return false;
        }
        now.saturating_sub(*self.since.get_or_insert(now)) >= Duration::from_secs(2)
    }
}
#[cfg(test)]
mod backlog_tests {
    use super::*;
    #[test]
    fn codec_pts_preserves_precise_repeated_and_reordered_tracking_timestamps() {
        let mut map = CodecFrameMap::default();
        let original = Duration::from_nanos(123456789);
        let first = map.insert(original);
        let repeated = map.insert(original);
        let reordered = map.insert(original - Duration::from_millis(20));
        assert!(first < repeated && repeated < reordered);
        assert_eq!(map.frames.remove(&first).unwrap().timestamp, original);
        assert_eq!(map.frames.remove(&repeated).unwrap().timestamp, original);
        assert_eq!(
            map.frames.remove(&reordered).unwrap().timestamp,
            original - Duration::from_millis(20)
        );
    }

    #[test]
    fn completed_outputs_do_not_count_as_decoder_input_backlog() {
        let mut map = CodecFrameMap::default();
        let first = map.insert(Duration::from_secs(1));
        map.insert(Duration::from_secs(2));
        assert_eq!(map.pending(), 2);
        map.frames.get_mut(&first).unwrap().output = Some(std::time::Instant::now());
        assert_eq!(map.pending(), 1);
    }
    #[test]
    fn transient_backlog_recovers_without_reset() {
        let mut w = BacklogWatchdog::default();
        assert!(!w.expired(Duration::ZERO, Duration::from_millis(200)));
        assert!(!w.expired(Duration::from_secs(1), Duration::ZERO));
        assert!(!w.expired(Duration::from_secs(3), Duration::from_millis(200)));
        assert!(!w.expired(Duration::from_secs(4), Duration::from_millis(200)));
        assert!(w.expired(Duration::from_secs(5), Duration::from_millis(200)));
    }
}
