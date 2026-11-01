//! Keep video pose timestamps separate from the Runtime display deadline.
use std::time::Duration;

pub(crate) fn select_display_time(
    is_vrx: bool,
    video_timestamp: Duration,
    predicted_display_time: Duration,
    custom_reprojection: bool,
) -> Duration {
    if is_vrx || custom_reprojection {
        predicted_display_time
    } else {
        video_timestamp.max(predicted_display_time.saturating_sub(Duration::from_secs(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vrx_uses_runtime_deadline_for_early_late_and_repeated_video() {
        let target = Duration::from_secs(10);
        for video_ms in [100, 9989, 10000, 10011] {
            assert_eq!(
                select_display_time(true, Duration::from_millis(video_ms), target, false),
                target
            );
        }
        assert_eq!(
            select_display_time(true, target, target + Duration::from_millis(11), false),
            target + Duration::from_millis(11)
        );
    }

    #[test]
    fn other_platforms_keep_existing_display_time_policy() {
        let target = Duration::from_secs(10);
        for (video_ms, expected_ms) in [(9989, 9989), (1000, 9000), (10011, 10011)] {
            assert_eq!(
                select_display_time(false, Duration::from_millis(video_ms), target, false),
                Duration::from_millis(expected_ms)
            );
        }
        assert_eq!(
            select_display_time(false, Duration::from_secs(1), target, true),
            target
        );
    }
}
