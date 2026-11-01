//! Instruction-derived controller from RVRPlugin 5.0.4, RVA 0x218f0/0x329e0.
//! The adapter supplies validated provider counts and per-eye records. Network
//! arrival times are used only for the long-window frequency estimate.
use alvr_packets::vrx_phase::{VRX_PHASE_VERSION, VrxPhaseMessage};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

/// Full-period or half-period target for a fixed virtual clock reading.
/// The conditional I-frame advance affects this wait only, not the saved grid.
pub(crate) fn deadline(
    previous: Option<Instant>,
    now: Instant,
    period: Duration,
    correction: i64,
    full: bool,
    pll: bool,
    advance: Duration,
) -> (Instant, Instant) {
    let mut saved = previous.unwrap_or_else(|| now - period / 2);
    if full {
        saved += period;
        while if pll {
            saved < now
        } else {
            saved + period <= now
        } {
            saved += period;
        }
        if pll {
            let step = Duration::from_nanos(correction.unsigned_abs());
            if correction < 0 {
                saved -= step;
            } else {
                saved += step;
            }
        }
    }
    (
        saved,
        saved + if full { Duration::ZERO } else { period / 2 } - advance,
    )
}

pub struct PhaseSession {
    controller: LenovoPhaseController,
    session: u64,
    token: u64,
    offered: Instant,
    last_feedback: Option<Instant>,
    sequence: u64,
    capable: bool,
    active: bool,
    frequency_only: bool,
    nominal: Duration,
    watermark: Option<u64>,
    diagnostic_ticks: Option<u64>,
}

impl PhaseSession {
    pub fn new(period: Duration) -> Self {
        let phase = Self {
            controller: LenovoPhaseController::new(period),
            session: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            token: 0,
            offered: Instant::now(),
            last_feedback: None,
            sequence: 0,
            capable: false,
            active: std::env::var("ALVR_VRX_LENOVO_PHASE").as_deref() == Ok("active"),
            frequency_only: std::env::var("ALVR_VRX_LENOVO_PHASE").as_deref() == Ok("frequency"),
            nominal: period,
            watermark: None,
            diagnostic_ticks: (std::env::var("ALVR_VRX_LENOVO_PHASE").as_deref()
                == Ok("diagnostic"))
            .then_some(0),
        };
        alvr_common::info!(
            "event=vrx_lenovo_session session={} active={}",
            phase.session,
            phase.active
        );
        phase
    }

    pub fn offer(&mut self) -> Option<String> {
        self.token = self.token.wrapping_add(1);
        self.offered = Instant::now();
        VrxPhaseMessage::Offer {
            version: VRX_PHASE_VERSION,
            session: self.session,
            token: self.token,
        }
        .encode()
    }

    pub fn receive(&mut self, value: &str) {
        let now = Instant::now();
        match VrxPhaseMessage::decode(value) {
            Some(VrxPhaseMessage::Capability {
                version,
                session,
                token,
                provider,
            }) if version == VRX_PHASE_VERSION
                && session == self.session
                && token == self.token =>
            {
                self.capable = provider;
                if !provider {
                    self.last_feedback = None;
                    self.controller = LenovoPhaseController::new(self.nominal);
                }
            }
            Some(VrxPhaseMessage::Feedback {
                version,
                session,
                token,
                sequence,
                vsync,
                duplicates,
                history,
            }) if self.capable
                && version == VRX_PHASE_VERSION
                && session == self.session
                && token == self.token
                && sequence > self.sequence
                && vsync != 0
                && history.atw.len() <= 10
                && history.atw.iter().all(|record| record.eye <= 1)
                && history.submits.iter().all(|eye| eye.len() <= 11)
                && now.duration_since(self.offered) < Duration::from_millis(1500) =>
            {
                self.sequence = sequence;
                let recent = self
                    .last_feedback
                    .is_some_and(|t| now.duration_since(t) < Duration::from_millis(250));
                if !recent && self.last_feedback.take().is_some() {
                    self.controller = LenovoPhaseController::new(self.nominal);
                }
                if self.watermark.is_some_and(|v| vsync < v) {
                    self.watermark = Some(vsync);
                    self.controller = LenovoPhaseController::new(self.nominal);
                    self.last_feedback = None;
                    return;
                }
                if !recent && self.watermark == Some(vsync) {
                    return;
                }
                let advances = self.watermark.is_some_and(|v| vsync > v);
                self.watermark = Some(vsync);
                let right_records = history.atw.iter().filter(|r| r.eye == 1).count();
                if history.dropped > 0 {
                    alvr_common::warn!(
                        "event=vrx_phase_history_overflow dropped={}",
                        history.dropped
                    );
                }
                if sequence % 90 == 1 {
                    alvr_common::info!(
                        "event=vrx_lenovo_received session={session} sequence={sequence} vc={vsync} dcl={} dcr={} active={}",
                        duplicates[0],
                        duplicates[1],
                        self.active
                    );
                }
                match self
                    .controller
                    .observe(now, vsync, duplicates, right_records)
                {
                    Observation::Progress if advances => self.last_feedback = Some(now),
                    Observation::Progress => (),
                    Observation::Reset => self.last_feedback = None,
                    Observation::Unchanged => (),
                }
            }
            _ => (),
        }
    }

    pub fn tick(&mut self, now: Instant) -> Option<(Duration, i64, bool)> {
        let correction = self.controller.render_tick(now);
        let fresh = self
            .last_feedback
            .is_some_and(|time| now.duration_since(time) < Duration::from_millis(250));
        if !fresh {
            if self.last_feedback.take().is_some() {
                alvr_common::info!("event=phase_fallback reason=stale_or_stalled session={}",self.session);
                self.controller = LenovoPhaseController::new(self.nominal);
            }
            return None;
        }
        if correction != 0 {
            alvr_common::info!(
                "event=vrx_lenovo_phase active={} correction_ns={correction}",
                self.active
            );
        }
        let injected = self.diagnostic_ticks.as_mut().map(|ticks| {
            *ticks += 1;
            match *ticks % 180 {
                60 => -1_000_000,
                120 => 1_000_000,
                _ => 0,
            }
        });
        (self.active || self.frequency_only || injected.is_some()).then_some((
            self.controller.period(),
            injected.unwrap_or(if self.active { correction } else { 0 }),
            self.controller.pll_enabled,
        ))
    }
}

pub struct LenovoPhaseController {
    period: Duration,
    frequency_anchor: Option<(u64, Instant)>,
    previous_counts: Option<(u64, [u64; 2])>,
    previous_duplicates: Option<u64>,
    previous_cost: u64,
    costs: VecDeque<u64>,
    reference_minimum: u64,
    multiplier: f32,
    records: usize,
    observing: bool,
    direction: f32,
    pending: Option<f32>,
    last_render: Option<Instant>,
    render_intervals: Vec<Duration>,
    pll_enabled: bool,
}

enum Observation {
    Progress,
    Unchanged,
    Reset,
}

impl LenovoPhaseController {
    pub fn new(period: Duration) -> Self {
        Self {
            period,
            frequency_anchor: None,
            previous_counts: None,
            previous_duplicates: None,
            previous_cost: 0,
            costs: VecDeque::with_capacity(10),
            reference_minimum: 0,
            multiplier: 1.0,
            records: 0,
            observing: false,
            direction: 1.0,
            pending: None,
            last_render: None,
            render_intervals: Vec::with_capacity(91),
            pll_enabled: false,
        }
    }

    pub fn period(&self) -> Duration {
        self.period
    }

    fn observe(
        &mut self,
        now: Instant,
        vsync: u64,
        duplicates: [u64; 2],
        right_records: usize,
    ) -> Observation {
        if let Some((previous_vsync, previous_duplicates)) = self.previous_counts {
            if vsync < previous_vsync {
                *self = Self::new(self.period);
                return Observation::Reset;
            }
            if duplicates
                .iter()
                .zip(previous_duplicates)
                .any(|(&new, old)| new < old)
            {
                *self = Self::new(self.period);
                return Observation::Reset;
            }
        }
        let new_boundary = self.previous_counts.is_none_or(|(prev, _)| prev != vsync);
        self.previous_counts = Some((vsync, duplicates));
        // The official source updates on exact 1000-counter boundaries.
        if new_boundary && vsync != 0 && vsync % 1000 == 0 {
            if let Some((previous, time)) = self.frequency_anchor
                && vsync.checked_sub(previous) == Some(1000)
            {
                let measured = now.saturating_duration_since(time).div_f64(1000.0);
                // Reject discontinuities instead of using an unbounded cadence.
                if measured >= Duration::from_millis(5) && measured <= Duration::from_millis(25) {
                    let elapsed_ns = now.duration_since(time).as_nanos() as f32;
                    let frequency = 1000.0f32 / (elapsed_ns / 1e9f32);
                    self.period = Duration::from_nanos((1e9f32 / frequency) as u64);
                }
            }
            self.frequency_anchor = Some((vsync, now));
        }
        if right_records == 0 || right_records > 11 {
            return Observation::Unchanged;
        }
        if !self.pll_enabled {
            self.records = 0;
            self.observing = false;
            self.previous_duplicates = Some(duplicates[0]);
            return Observation::Progress;
        }
        self.records += right_records;
        let window = if self.observing { 5 } else { 50 };
        if self.records < window {
            return Observation::Progress;
        }
        self.records = 0;
        if self.observing {
            self.observing = false;
            return Observation::Progress;
        }
        let cost = self
            .previous_duplicates
            .filter(|value| *value != 0)
            .map_or(0, |previous| duplicates[0] - previous);
        self.previous_duplicates = Some(duplicates[0]);
        self.costs.push_back(cost);
        let minimum = if self.costs.len() > 10 {
            self.costs.pop_front();
            self.costs.iter().copied().min().unwrap_or(0)
        } else {
            0
        };
        let correction = self.correction(cost, minimum, window as u64);
        self.reference_minimum = minimum;
        self.pending = Some(correction);
        alvr_common::info!(
            "event=vrx_lenovo_candidate repeats={cost} minimum={minimum} correction={correction} correction_ns={}",
            Self::correction_ns(correction)
        );
        self.previous_cost = cost;
        self.observing = true;
        Observation::Progress
    }

    fn correction(&mut self, cost: u64, minimum: u64, window: u64) -> f32 {
        let multiplier = if cost == self.previous_cost { 0.5 } else { 1.0 };
        self.multiplier = multiplier;
        if cost > self.previous_cost {
            self.direction *= -1.0;
        }
        let excess = cost.saturating_sub(minimum);
        // R8 is unsigned after SUB at 0x180022a10. Duplicates from the five
        // observation records can put the mature minimum above fifty.
        let range = window.wrapping_sub(minimum);
        // Preserve the actual unsigned-integer threshold divisions, including
        // the reciprocal-multiply expression in the /7 branch.
        let high = ((u128::from(range) * 0x2492492492492493) >> 64) as u64;
        let threshold_seven = ((range - high) / 2 + high) / 4;
        let magnitude = if excess > range / 5 {
            self.direction = -1.0;
            2.0
        } else if excess > threshold_seven {
            self.direction = -1.0;
            1.33
        } else if excess > range / 10 {
            self.direction = -1.0;
            0.67
        } else if excess > range / 20 {
            0.67
        } else if excess != 0 {
            0.5
        } else {
            self.direction = 1.0;
            0.1
        };
        magnitude * self.direction * multiplier
    }

    pub fn render_tick(&mut self, now: Instant) -> i64 {
        if let Some(previous) = self.last_render {
            self.render_intervals
                .push(now.saturating_duration_since(previous));
            if self.render_intervals.len() > 90 {
                let mean = self.render_intervals.iter().copied().sum::<Duration>() / 91;
                self.pll_enabled = mean <= self.period.mul_f64(1.025);
                self.render_intervals.clear();
            }
        }
        self.last_render = Some(now);
        let correction = self.pending.take().unwrap_or(0.0);
        if self.pll_enabled {
            Self::correction_ns(correction)
        } else {
            0
        }
    }

    fn correction_ns(correction: f32) -> i64 {
        // cvttss2si precedes imul 1000000 in the original instructions.
        (correction as i64) * 1_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_packets_match_official_state_transitions() {
        let mut ctl = LenovoPhaseController::new(Duration::from_nanos(11_111_111));
        let epoch = Instant::now();
        for (i, row) in include_str!("../../../experiments/phase/trajectory.tsv")
            .lines()
            .enumerate()
        {
            let v: Vec<i64> = row.split_whitespace().map(|x| x.parse().unwrap()).collect();
            ctl.pll_enabled = v[4] != 0;
            ctl.observe(
                epoch + Duration::from_nanos(v[0] as u64),
                v[1] as u64,
                [v[2] as u64; 2],
                v[3] as usize,
            );
            let actual = [
                ctl.records as i64,
                ctl.observing as i64,
                ctl.previous_cost as i64,
                ctl.reference_minimum as i64,
                ctl.direction.to_bits() as i64,
                ctl.multiplier.to_bits() as i64,
                ctl.pending.take().map_or(-1, |x| x.to_bits() as i64),
                ctl.period.as_nanos() as i64,
            ];
            assert_eq!(
                &actual[..],
                &v[5..],
                "first divergence row {i}, 0x180022613..0x180022ca2 input {row}"
            );
        }
    }

    #[test]
    fn deadlines_match_virtual_clock_dll_calls() {
        let epoch = Instant::now();
        for row in include_str!("../../../experiments/phase/wait.tsv").lines() {
            let v: Vec<i64> = row.split_whitespace().map(|n| n.parse().unwrap()).collect();
            let at = |n| epoch + Duration::from_nanos(n as u64);
            let (saved, target) = deadline(
                (v[0] != 0).then(|| at(v[0])),
                at(v[1]),
                Duration::from_nanos(11_111_111),
                v[2] * 1_000_000,
                v[3] != 0,
                v[4] != 0,
                Duration::from_nanos(v[5] as u64),
            );
            assert_eq!(
                (target, saved),
                (at(v[6]), at(v[7])),
                "0x180032dd6 input {row}"
            );
        }
    }

    #[test]
    fn matches_dll_cost_float_bits_and_integer_step() {
        for row in include_str!("../../../experiments/phase/oracle.tsv").lines() {
            let v: Vec<i64> = row.split_whitespace().map(|n| n.parse().unwrap()).collect();
            let mut ctl = LenovoPhaseController::new(Duration::from_millis(11));
            ctl.pll_enabled = true;
            ctl.previous_duplicates = Some(v[0] as u64);
            ctl.previous_cost = v[2] as u64;
            ctl.costs.extend([v[3] as u64; 10]);
            ctl.direction = v[4] as f32;
            ctl.records = 49;
            ctl.observe(Instant::now(), 100, [v[1] as u64; 2], 1);
            let raw = ctl.pending.unwrap();
            assert_eq!(ctl.previous_cost, v[5] as u64, "0x180022876 input {row}");
            assert_eq!(raw.to_bits(), v[6] as u32, "0x180022b29 input {row}");
            assert_eq!(
                LenovoPhaseController::correction_ns(raw),
                v[7],
                "0x180032d11 input {row}"
            );
        }
    }

    #[test]
    fn collection_and_observation_windows_follow_50_then_5() {
        let epoch = Instant::now();
        let mut controller = LenovoPhaseController::new(Duration::from_millis(10));
        controller.pll_enabled = true;
        controller.previous_duplicates = Some(1);
        for i in 1..50 {
            controller.observe(epoch, i, [1, 1], 1);
        }
        assert!(controller.pending.is_none());
        controller.observe(epoch, 50, [12, 12], 1);
        assert_eq!(controller.pending, Some(-2.0));
        assert!(controller.observing);
        controller.pending = None;
        for i in 51..=55 {
            controller.observe(epoch, i, [12, 12], 1);
        }
        assert!(!controller.observing);
        assert!(controller.pending.is_none());
        controller.observe(epoch, 1, [0, 0], 1);
        assert!(controller.previous_counts.is_none());
    }

    #[test]
    fn feedback_requires_current_session_token_capability_and_sequence() {
        let mut phase = PhaseSession::new(Duration::from_millis(10));
        phase.offer();
        let encode = |session, token, sequence| {
            VrxPhaseMessage::Feedback {
                version: VRX_PHASE_VERSION,
                session,
                token,
                sequence,
                vsync: 100,
                duplicates: [0, 0],
                history: alvr_packets::vrx_phase::History {
                    atw: [0, 1]
                        .map(|eye| alvr_packets::vrx_phase::VrxEyeRecord {
                            image: Default::default(),
                            eye,
                            video_timestamp_ns: 1,
                            submission: sequence,
                            submit_to_wait_ns: 1,
                        })
                        .to_vec(),
                    ..Default::default()
                },
            }
            .encode()
            .unwrap()
        };
        let sample = encode(phase.session, phase.token, 1);
        phase.receive(&sample);
        assert_eq!(phase.sequence, 0);
        phase.capable = true;
        phase.receive(&encode(phase.session + 1, phase.token, 1));
        phase.receive(&encode(phase.session, phase.token + 1, 1));
        assert_eq!(phase.sequence, 0);
        phase.receive(&sample);
        assert_eq!(phase.sequence, 1);
        let last_progress = phase.last_feedback;
        phase.receive(&encode(phase.session, phase.token, 2));
        assert_eq!(
            phase.last_feedback, last_progress,
            "frozen provider count must not renew freshness"
        );
        phase.receive(&sample);
        assert_eq!(phase.sequence, 2);
        phase.last_feedback = Some(Instant::now() - Duration::from_secs(1));
        assert!(phase.tick(Instant::now()).is_none());
        phase.active = true;
        for sequence in 3..10 {
            phase.receive(&encode(phase.session, phase.token, sequence));
            assert!(phase.tick(Instant::now()).is_none());
        }
    }

    #[test]
    fn native_conversion_truncates_before_scaling() {
        for (value, expected) in [
            (0.1, 0),
            (-0.67, 0),
            (1.33, 1_000_000),
            (-1.33, -1_000_000),
            (-2.0, -2_000_000),
        ] {
            assert_eq!(LenovoPhaseController::correction_ns(value), expected);
        }
    }

    #[test]
    fn thresholds_and_direction_match_native_branches() {
        let mut controller = LenovoPhaseController::new(Duration::from_millis(10));
        for (cost, expected) in [
            (0, 0.05),
            (1, -0.5),
            (3, -0.67),
            (6, -0.67),
            (8, -1.33),
            (11, -2.0),
        ] {
            controller.direction = 1.0;
            controller.previous_cost = 0;
            assert!((controller.correction(cost, 0, 50) - expected).abs() < 0.00001);
        }
    }

    #[test]
    fn overload_suppresses_pending_correction() {
        let mut controller = LenovoPhaseController::new(Duration::from_millis(10));
        let epoch = Instant::now();
        for i in 0..=91 {
            controller.render_tick(epoch + Duration::from_millis(i * 10));
        }
        assert!(controller.pll_enabled);
        controller.pending = Some(-2.0);
        assert_eq!(
            controller.render_tick(epoch + Duration::from_millis(920)),
            -2_000_000
        );
        for i in 1..=91 {
            controller.render_tick(epoch + Duration::from_millis(920 + i * 20));
        }
        controller.pending = Some(-2.0);
        assert_eq!(
            controller.render_tick(epoch + Duration::from_millis(2760)),
            0
        );
    }

    #[test]
    fn frequency_updates_only_at_complete_thousand_counter_boundaries() {
        let epoch = Instant::now();
        let mut controller = LenovoPhaseController::new(Duration::from_millis(10));
        controller.observe(epoch, 1000, [0, 0], 1);
        controller.observe(epoch + Duration::from_secs(11), 1999, [0, 0], 1);
        assert_eq!(controller.period(), Duration::from_millis(10));
        controller.observe(epoch + Duration::from_secs(12), 2000, [0, 0], 1);
        assert_eq!(controller.period(), Duration::from_millis(12));
        // Missing an entire boundary does not turn a 2000-tick span into 1000.
        controller.observe(epoch + Duration::from_secs(36), 4000, [0, 0], 1);
        assert_eq!(controller.period(), Duration::from_millis(12));
    }

    #[test]
    fn unnegotiated_or_expired_feedback_always_falls_back_even_in_active_mode() {
        let mut phase = PhaseSession::new(Duration::from_millis(10));
        phase.active = true;
        assert!(phase.tick(Instant::now()).is_none());
        phase.capable = true;
        phase.last_feedback = Some(Instant::now());
        assert!(phase.tick(Instant::now()).is_some());
        phase.last_feedback = Some(Instant::now() - Duration::from_millis(251));
        assert!(phase.tick(Instant::now()).is_none());
        assert!(phase.controller.pending.is_none());
    }
}
