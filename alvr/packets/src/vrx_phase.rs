use serde::{Deserialize, Serialize};

pub const VRX_PHASE_VERSION: u32 = 2;
pub const VRX_PHASE_PREFIX: &str = "alvr.vrx.phase:";

/// OpenXR application submissions, not compositor selection or scanout evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VrxEyeRecord {
    pub eye: u8,
    pub video_timestamp_ns: u64,
    pub submission: u64,
    pub submit_to_wait_ns: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitRecord {
    pub submission: u64,
    pub video_timestamp_ns: u64,
    pub submit_ns: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct History {
    pub atw: Vec<VrxEyeRecord>,
    pub submits: [Vec<SubmitRecord>; 2],
    pub dropped: u64,
}

impl History {
    pub fn append(&mut self, other: &mut Self) {
        fn append<T>(dst: &mut Vec<T>, src: &mut Vec<T>, limit: usize) -> u64 {
            dst.append(src);
            let excess = dst.len().saturating_sub(limit);
            dst.drain(..excess);
            excess as u64
        }
        self.dropped += other.dropped + append(&mut self.atw, &mut other.atw, 10);
        for (dst, src) in self.submits.iter_mut().zip(&mut other.submits) {
            self.dropped += append(dst, src, 11);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum VrxPhaseMessage {
    Offer {
        version: u32,
        session: u64,
        token: u64,
    },
    Capability {
        version: u32,
        session: u64,
        token: u64,
        provider: bool,
    },
    Feedback {
        version: u32,
        session: u64,
        token: u64,
        sequence: u64,
        vsync: u64,
        duplicates: [u64; 2],
        history: History,
    },
}

impl VrxPhaseMessage {
    pub fn encode(&self) -> Option<String> {
        serde_json::to_string(self)
            .ok()
            .map(|json| format!("{VRX_PHASE_PREFIX}{json}"))
    }

    pub fn decode(value: &str) -> Option<Self> {
        if value.len() > 8192 {
            return None;
        }
        serde_json::from_str(value.strip_prefix(VRX_PHASE_PREFIX)?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_sender_preserves_three_submissions_and_bounds_each_history() {
        let mut batch = History::default();
        for submission in 1..=12 {
            batch.append(&mut History {
                atw: [0, 1]
                    .map(|eye| VrxEyeRecord {
                        eye,
                        submission,
                        video_timestamp_ns: 42,
                        submit_to_wait_ns: 100,
                    })
                    .to_vec(),
                submits: [0, 1].map(|_| {
                    vec![SubmitRecord {
                        submission,
                        video_timestamp_ns: 42,
                        submit_ns: 100,
                    }]
                }),
                dropped: 0,
            });
            if submission == 3 {
                assert_eq!(batch.atw.iter().filter(|r| r.eye == 1).count(), 3);
                assert_eq!(batch.submits[0].len(), 3);
            }
        }
        assert_eq!(batch.atw.len(), 10);
        assert_eq!(batch.submits[1].len(), 11);
        assert_eq!(batch.submits[1][0].submission, 2);
        assert_eq!(batch.dropped, 16);
    }
}
