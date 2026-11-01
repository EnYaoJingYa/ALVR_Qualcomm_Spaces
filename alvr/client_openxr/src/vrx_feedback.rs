
use alvr_client_core::ClientCoreContext;
use alvr_common::{parking_lot::Mutex, RelaxedAtomic};
use alvr_packets::vrx_phase::{SubmissionHistory, VrxPhaseMessage, VRX_PHASE_VERSION};
use std::{sync::Arc, thread::{self, JoinHandle}, time::{Duration, Instant}};

pub struct Feedback {
    pub history: Arc<Mutex<SubmissionHistory>>,
    pub epoch: Instant,
    running: Arc<RelaxedAtomic>,
    worker: Option<JoinHandle<()>>,
}
impl Feedback {
    pub fn new(core: Arc<ClientCoreContext>) -> Self {
        let history = Arc::new(Mutex::new(SubmissionHistory::default()));
        let running = Arc::new(RelaxedAtomic::new(true));
        let epoch = Instant::now();
        let h = history.clone();
        let active = running.clone();
        let worker = thread::Builder::new().name("VrxVsyncFeedback".into()).spawn(move || {
            let mut session = None;
            let mut counter = None;
            let mut previous = None;
            let mut capability = None;
            let mut sequence = 0;
            while active.value() {
                thread::sleep(Duration::from_millis(2));
                let Some((current, token)) = core.vrx_phase_offer() else { continue };
                if session != Some(current) {
                    counter = crate::vrx_counter::VrxCounter::open();
                    session = Some(current);
                    previous = None;
                    capability = None;
                    sequence = 0;
                    *h.lock() = SubmissionHistory::default();
                }
                let count = counter.as_mut().and_then(crate::vrx_counter::VrxCounter::read);
                let provider = count.is_some();
                if capability != Some((token, provider)) {
                    core.send_vrx_phase(VrxPhaseMessage::Capability {
                        version: VRX_PHASE_VERSION, session: current, token, provider,
                    });
                    capability = Some((token, provider));
                }
                let Some(vsync) = count else { continue };
                if previous == Some(vsync) { continue }
                let skipped = previous.map_or(0, |old: u64| vsync.saturating_sub(old).saturating_sub(1));
                previous = Some(vsync);
                let mut state = h.lock();
                let duplicates = state.duplicates;
                let submissions = state.submissions;
                let history = std::mem::take(&mut state.history);
                drop(state);
                sequence += 1;
                if sequence % 90 == 1 || skipped > 0 {
                    alvr_common::info!("event=vrx_vsync_feedback seq={sequence} vc={vsync} skipped={skipped} submissions={submissions} dcl={} dcr={} sample_ns={}",
                        duplicates[0], duplicates[1], epoch.elapsed().as_nanos());
                }
                core.send_vrx_phase(VrxPhaseMessage::Feedback {
                    version: VRX_PHASE_VERSION, session: current, token, sequence,
                    vsync, duplicates, history,
                });
            }
        }).map_err(|error| {
            crate::StageFailure::new("phase_feedback_worker", "thread_spawn", true,
                "nominal_frame_pacing", "instance,session", error.to_string())
                .log(alvr_system_info::Platform::ThinkRealityVRX);
        }).ok();
        Self { history, epoch, running, worker }
    }
}
impl Drop for Feedback {
    fn drop(&mut self) {
        self.running.set(false);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                alvr_common::error!("event=vrx_phase_failed stage=worker_join code=thread_failure retryable=true fallback=nominal_frame_pacing resources=released");
            }
        }
    }
}
