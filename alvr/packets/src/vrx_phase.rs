use serde::{Deserialize, Serialize};

pub const VRX_PHASE_VERSION: u32 = 3;
pub const VRX_PHASE_PREFIX: &str = "alvr.vrx.phase:";

/// Local decoded-image identity. Tracking predictions and buffer addresses can repeat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageIdentity {
    pub decoder: u64,
    pub image: u64,
}

/// OpenXR application submissions, not compositor selection or scanout evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VrxEyeRecord {
    pub eye: u8,
    pub image: ImageIdentity,
    pub video_timestamp_ns: u64,
    pub submission: u64,
    pub submit_to_wait_ns: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitRecord {
    pub image: ImageIdentity,
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

/// Successful application submissions, independently drained by the VSync sender.
#[derive(Default)]
pub struct SubmissionHistory {
    pub duplicates: [u64; 2],
    pub submissions: u64,
    previous_image: Option<ImageIdentity>,
    awaiting_wait: Option<SubmitRecord>,
    pub history: History,
}

impl SubmissionHistory {
    pub fn confirm(&mut self, image: ImageIdentity, timestamp: u64, now: u64) {
        if self.previous_image == Some(image) {
            self.duplicates[0] += 1;
            self.duplicates[1] += 1;
        }
        self.previous_image = Some(image);
        self.submissions += 1;
        let record = SubmitRecord {
            image,
            submission: self.submissions,
            video_timestamp_ns: timestamp,
            submit_ns: now,
        };
        self.awaiting_wait = Some(record.clone());
        self.history.append(&mut History {
            submits: [vec![record.clone()], vec![record]],
            ..Default::default()
        });
    }

    pub fn after_wait(&mut self, now: u64) {
        if let Some(record) = self.awaiting_wait.take() {
            self.history.append(&mut History {
                atw: [0, 1].map(|eye| VrxEyeRecord {
                    eye,
                    image: record.image,
                    submission: record.submission,
                    video_timestamp_ns: record.video_timestamp_ns,
                    submit_to_wait_ns: now.saturating_sub(record.submit_ns),
                }).to_vec(),
                ..Default::default()
            });
        }
    }
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
    fn fresh_images_with_same_tracking_time_and_decoder_restart_are_distinct() {
        let mut state = SubmissionHistory::default();
        for image in 1..=2 {
            state.confirm(ImageIdentity { decoder: 1, image }, 42, image * 10);
            state.after_wait(image * 10 + 5);
        }
        assert_eq!(state.duplicates, [0, 0]);
        let same = ImageIdentity { decoder: 1, image: 2 };
        state.confirm(same, 42, 30);
        state.after_wait(35);
        state.confirm(same, 42, 40);
        state.after_wait(45);
        assert_eq!(state.duplicates, [2, 2]);
        let batch = std::mem::take(&mut state.history);
        assert_eq!(batch.atw.len(), 8);
        assert_eq!(batch.submits[0].len(), 4);
        state.confirm(ImageIdentity { decoder: 2, image: 2 }, 42, 50);
        assert_eq!(state.duplicates, [2, 2]);
    }

    #[test]
    fn delayed_sender_preserves_three_submissions_and_bounds_each_history() {
        let mut batch = History::default();
        for submission in 1..=12 {
            batch.append(&mut History {
                atw: [0, 1]
                    .map(|eye| VrxEyeRecord {
                        eye,
                        image: ImageIdentity { decoder: 1, image: submission },
                        submission,
                        video_timestamp_ns: 42,
                        submit_to_wait_ns: 100,
                    })
                    .to_vec(),
                submits: [0, 1].map(|_| {
                    vec![SubmitRecord {
                        image: ImageIdentity { decoder: 1, image: submission },
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
