use alvr_common::{HEAD_ID, SlidingWindowAverage};
use alvr_events::{BitrateDirectives, EventType, GraphStatistics, StatisticsSummary};
use alvr_packets::ClientStatistics;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

const FULL_REPORT_INTERVAL: Duration = Duration::from_millis(500);
const EPS_INTERVAL: Duration = Duration::from_micros(1);

pub struct HistoryFrame {
    target_timestamp: Duration,
    tracking_received: Instant,
    frame_present: Instant,
    frame_composed: Instant,
    frame_encoded: Instant,
    video_packet_bytes: usize,
    total_pipeline_latency: Duration,
}

impl Default for HistoryFrame {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            target_timestamp: Duration::ZERO,
            tracking_received: now,
            frame_present: now,
            frame_composed: now,
            frame_encoded: now,
            video_packet_bytes: 0,
            total_pipeline_latency: Duration::ZERO,
        }
    }
}

#[derive(Default, Clone)]
struct BatteryData {
    gauge_value: f32,
    is_plugged: bool,
}

pub struct StatisticsManager {
    history_buffer: VecDeque<HistoryFrame>,
    max_history_size: usize,
    last_full_report_instant: Instant,
    last_frame_present_instant: Instant,
    last_frame_present_interval: Duration,
    video_packets_total: usize,
    video_packets_partial_sum: usize,
    video_bytes_total: usize,
    video_bytes_partial_sum: usize,
    battery_gauges: HashMap<u64, BatteryData>,
    steamvr_pipeline_latency: Duration,
    motion_to_photon_latency_average: SlidingWindowAverage<Duration>,
    last_vsync_time: Instant,
    vrx_phase: Option<crate::vrx_phase::PhaseSession>,
    frame_interval: Duration,
    nominal_frame_interval: Duration,
    phase_deadline: Option<Instant>,
    last_throughput_directives: BitrateDirectives,
}

impl StatisticsManager {
    // history size used to calculate average total pipeline latency
    pub fn new(
        max_history_size: usize,
        nominal_server_frame_interval: Duration,
        steamvr_pipeline_frames: f32,
    ) -> Self {
        Self {
            history_buffer: VecDeque::new(),
            max_history_size,
            last_full_report_instant: Instant::now(),
            last_frame_present_instant: Instant::now(),
            last_frame_present_interval: Duration::ZERO,
            video_packets_total: 0,
            video_packets_partial_sum: 0,
            video_bytes_total: 0,
            video_bytes_partial_sum: 0,
            battery_gauges: HashMap::new(),
            steamvr_pipeline_latency: Duration::from_secs_f32(
                steamvr_pipeline_frames * nominal_server_frame_interval.as_secs_f32(),
            ),
            motion_to_photon_latency_average: SlidingWindowAverage::new(
                Duration::ZERO,
                max_history_size,
            ),
            last_vsync_time: Instant::now(),
            vrx_phase: None,
            phase_deadline: None,
            frame_interval: nominal_server_frame_interval,
            nominal_frame_interval: nominal_server_frame_interval,
            last_throughput_directives: BitrateDirectives::default(),
        }
    }

    pub fn enable_lenovo_phase(&mut self) {
        self.vrx_phase = Some(crate::vrx_phase::PhaseSession::new(self.frame_interval));
    }

    pub fn vrx_phase_offer(&mut self) -> Option<String> {
        self.vrx_phase.as_mut().and_then(|phase| phase.offer())
    }

    pub fn receive_vrx_phase(&mut self, value: &str) {
        if let Some(phase) = &mut self.vrx_phase {
            phase.receive(value);
        }
    }

    pub fn report_tracking_received(&mut self, target_timestamp: Duration) {
        if !self
            .history_buffer
            .iter()
            .any(|frame| frame.target_timestamp == target_timestamp)
        {
            self.history_buffer.push_front(HistoryFrame {
                target_timestamp,
                tracking_received: Instant::now(),
                ..Default::default()
            });
        }

        if self.history_buffer.len() > self.max_history_size {
            self.history_buffer.pop_back();
        }
    }

    pub fn report_frame_present(&mut self, target_timestamp: Duration, offset: Duration) {
        if let Some(frame) = self
            .history_buffer
            .iter_mut()
            .find(|frame| frame.target_timestamp == target_timestamp)
        {
            let now = Instant::now() - offset;

            self.last_frame_present_interval =
                now.saturating_duration_since(self.last_frame_present_instant);
            self.last_frame_present_instant = now;

            frame.frame_present = now;
        }
    }

    pub fn report_frame_composed(&mut self, target_timestamp: Duration, offset: Duration) {
        if let Some(frame) = self
            .history_buffer
            .iter_mut()
            .find(|frame| frame.target_timestamp == target_timestamp)
        {
            frame.frame_composed = Instant::now() - offset;
        }
    }

    // returns encoding interval
    pub fn report_frame_encoded(
        &mut self,
        target_timestamp: Duration,
        bytes_count: usize,
    ) -> Duration {
        self.video_packets_total += 1;
        self.video_packets_partial_sum += 1;
        self.video_bytes_total += bytes_count;
        self.video_bytes_partial_sum += bytes_count;

        if let Some(frame) = self
            .history_buffer
            .iter_mut()
            .find(|frame| frame.target_timestamp == target_timestamp)
        {
            frame.frame_encoded = Instant::now();

            frame.video_packet_bytes = bytes_count;

            frame
                .frame_encoded
                .saturating_duration_since(frame.frame_composed)
        } else {
            Duration::ZERO
        }
    }

    pub fn report_battery(&mut self, device_id: u64, gauge_value: f32, is_plugged: bool) {
        *self.battery_gauges.entry(device_id).or_default() = BatteryData {
            gauge_value,
            is_plugged,
        };
    }

    pub fn report_throughput_stats(&mut self, stats: BitrateDirectives) {
        self.last_throughput_directives = stats;
    }

    // Called every frame. Some statistics are reported once every frame
    // Returns (network latency, game time latency)
    pub fn report_statistics(&mut self, client_stats: ClientStatistics) -> (Duration, Duration) {
        // vsync_queue is measured before the client sends this feedback. Adding it to
        // the server's receive time would include transport and scheduling delays in
        // the display phase. Keep nominal pacing until a calibrated clock mapping is
        // available; feedback timestamps identify video frames, not server deadlines.
        self.motion_to_photon_latency_average
            .submit_sample(client_stats.total_pipeline_latency);

        if let Some(frame) = self
            .history_buffer
            .iter_mut()
            .find(|frame| frame.target_timestamp == client_stats.target_timestamp)
        {
            frame.total_pipeline_latency = client_stats.total_pipeline_latency;

            let game_time_latency = frame
                .frame_present
                .saturating_duration_since(frame.tracking_received);

            let server_compositor_latency = frame
                .frame_composed
                .saturating_duration_since(frame.frame_present);

            let encoder_latency = frame
                .frame_encoded
                .saturating_duration_since(frame.frame_composed);

            // The network latency cannot be estiamed directly. It is what's left of the total
            // latency after subtracting all other latency intervals. In particular it contains the
            // transport latency of the tracking packet and the interval between the first video
            // packet is sent and the last video packet is received for a specific frame.
            // For safety, use saturating_sub to avoid a crash if for some reason the network
            // latency is miscalculated as negative.
            let network_latency = frame.total_pipeline_latency.saturating_sub(
                game_time_latency
                    + server_compositor_latency
                    + encoder_latency
                    + client_stats.video_decode
                    + client_stats.video_decoder_queue
                    + client_stats.rendering
                    + client_stats.vsync_queue,
            );

            let client_fps =
                1.0 / Duration::max(client_stats.frame_interval, EPS_INTERVAL).as_secs_f32();
            let server_fps =
                1.0 / Duration::max(self.last_frame_present_interval, EPS_INTERVAL).as_secs_f32();

            if self.last_full_report_instant + FULL_REPORT_INTERVAL < Instant::now() {
                self.last_full_report_instant += FULL_REPORT_INTERVAL;

                let interval_secs = FULL_REPORT_INTERVAL.as_secs_f32();

                alvr_events::send_event(EventType::StatisticsSummary(StatisticsSummary {
                    video_packets_total: self.video_packets_total,
                    video_packets_per_sec: (self.video_packets_partial_sum as f32 / interval_secs)
                        as _,
                    video_mbytes_total: (self.video_bytes_total as f32 / 1e6) as usize,
                    video_mbits_per_sec: self.video_bytes_partial_sum as f32 * 8.
                        / 1e6
                        / interval_secs,
                    total_latency_ms: client_stats.total_pipeline_latency.as_secs_f32() * 1000.,
                    network_latency_ms: network_latency.as_secs_f32() * 1000.,
                    encode_latency_ms: encoder_latency.as_secs_f32() * 1000.,
                    decode_latency_ms: client_stats.video_decode.as_secs_f32() * 1000.,
                    client_fps: client_fps as _,
                    server_fps: server_fps as _,
                    battery_hmd: (self
                        .battery_gauges
                        .get(&HEAD_ID)
                        .cloned()
                        .unwrap_or_default()
                        .gauge_value
                        * 100.) as u32,
                    hmd_plugged: self
                        .battery_gauges
                        .get(&HEAD_ID)
                        .cloned()
                        .unwrap_or_default()
                        .is_plugged,
                }));

                self.video_packets_partial_sum = 0;
                self.video_bytes_partial_sum = 0;
            }

            let packet_bits = frame.video_packet_bytes as f32 * 8.0;
            let throughput_bps =
                packet_bits / Duration::max(network_latency, EPS_INTERVAL).as_secs_f32();
            let bitrate_bps = packet_bits
                / Duration::max(self.last_frame_present_interval, EPS_INTERVAL).as_secs_f32();

            // todo: use target timestamp in nanoseconds. the dashboard needs to use the first
            // timestamp as the graph time origin.
            alvr_events::send_event(EventType::GraphStatistics(GraphStatistics {
                total_pipeline_latency_s: client_stats.total_pipeline_latency.as_secs_f32(),
                game_time_s: game_time_latency.as_secs_f32(),
                server_compositor_s: server_compositor_latency.as_secs_f32(),
                encoder_s: encoder_latency.as_secs_f32(),
                network_s: network_latency.as_secs_f32(),
                decoder_s: client_stats.video_decode.as_secs_f32(),
                decoder_queue_s: client_stats.video_decoder_queue.as_secs_f32(),
                client_compositor_s: client_stats.rendering.as_secs_f32(),
                vsync_queue_s: client_stats.vsync_queue.as_secs_f32(),
                client_fps,
                server_fps,
                bitrate_directives: self.last_throughput_directives.clone(),
                throughput_bps,
                bitrate_bps,
            }));

            (network_latency, game_time_latency)
        } else {
            (Duration::ZERO, Duration::ZERO)
        }
    }

    pub fn motion_to_photon_latency_average(&self) -> Duration {
        self.motion_to_photon_latency_average.get_average()
    }

    pub fn tracker_pose_time_offset(&self) -> Duration {
        // This is the opposite of the client's StatisticsManager::tracker_prediction_offset().
        self.steamvr_pipeline_latency
    }

    // NB: this call is non-blocking, waiting should be done externally
    pub fn duration_until_next_vsync(&mut self) -> Duration {
        let now = Instant::now();

        if let Some(phase) = &mut self.vrx_phase {
            if let Some((period, correction, pll)) = phase.tick(now) {
                // Packed stereo consumes the full-period branch once per Present.
                let (saved, target) = crate::vrx_phase::deadline(
                    self.phase_deadline,
                    now,
                    period,
                    correction,
                    true,
                    pll,
                    Duration::ZERO,
                );
                self.phase_deadline = Some(saved);
                if let Some(t) = crate::vrx_diagnostic_time_ns() {
                    let delta = if target >= now {
                        (target - now).as_nanos() as i64
                    } else {
                        -((now - target).as_nanos() as i64)
                    };
                    let step = if pll { correction } else { 0 };
                    alvr_common::info!(
                        "event=phase_wait_target pc_ns={t} before_delta_ns={} after_delta_ns={delta} step_ns={step}",
                        delta - step
                    );
                }
                return target.saturating_duration_since(now);
            }
            if self.phase_deadline.take().is_some() {
                self.last_vsync_time = now;
            }
            self.frame_interval = self.nominal_frame_interval;
        }
        // update the last vsync if it's too old
        while self.last_vsync_time + self.frame_interval < now {
            self.last_vsync_time += self.frame_interval;
        }

        (self.last_vsync_time + self.frame_interval).saturating_duration_since(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_frame(stats: &mut StatisticsManager, timestamp: Duration, acquired: Instant) {
        stats.history_buffer.push_front(HistoryFrame {
            target_timestamp: timestamp,
            tracking_received: acquired,
            frame_present: acquired + Duration::from_millis(2),
            frame_composed: acquired + Duration::from_millis(3),
            frame_encoded: acquired + Duration::from_millis(6),
            ..Default::default()
        });
    }

    fn feedback(timestamp: Duration, display_lead: Duration) -> ClientStatistics {
        ClientStatistics {
            target_timestamp: timestamp,
            frame_interval: Duration::from_millis(10),
            video_decode: Duration::from_millis(4),
            video_decoder_queue: Duration::from_millis(1),
            rendering: Duration::from_millis(1),
            vsync_queue: display_lead,
            total_pipeline_latency: Duration::from_millis(20) + display_lead,
        }
    }

    #[test]
    fn uncalibrated_feedback_cannot_shift_the_server_pacing_grid() {
        let mut stats = StatisticsManager::new(256, Duration::from_millis(10), 0.0);
        let original_tick = stats.last_vsync_time;
        let original_interval = stats.frame_interval;
        // A backlog can deliver many old frames at once. All arrivals may have the
        // same nonzero transport bias: smoothing must not turn that into a new phase.
        let acquired = original_tick - Duration::from_secs(1);
        for i in 1..=128 {
            let timestamp = Duration::from_millis(i);
            add_frame(&mut stats, timestamp, acquired);
            let lead = Duration::from_millis(if i % 2 == 0 { 33 } else { 35 });
            stats.report_statistics(feedback(timestamp, lead));
            assert_eq!(stats.last_vsync_time, original_tick);
            assert_eq!(stats.frame_interval, original_interval);
        }
    }

    #[test]
    fn repeated_and_reordered_feedback_preserves_latency_accounting() {
        let mut stats = StatisticsManager::new(256, Duration::from_millis(10), 0.0);
        let original_tick = stats.last_vsync_time;
        let acquired = original_tick - Duration::from_secs(1);
        for timestamp_ms in [1, 2] {
            add_frame(&mut stats, Duration::from_millis(timestamp_ms), acquired);
        }
        for timestamp_ms in [2, 2, 1, 2] {
            let timestamp = Duration::from_millis(timestamp_ms);
            for lead_ms in [30, 32, 60] {
                let latencies =
                    stats.report_statistics(feedback(timestamp, Duration::from_millis(lead_ms)));
                assert_eq!(
                    latencies,
                    (Duration::from_millis(8), Duration::from_millis(2))
                );
                assert_eq!(stats.last_vsync_time, original_tick);
                assert_eq!(
                    stats
                        .history_buffer
                        .iter()
                        .find(|frame| frame.target_timestamp == timestamp)
                        .unwrap()
                        .total_pipeline_latency,
                    Duration::from_millis(20 + lead_ms)
                );
            }
        }
    }
}
