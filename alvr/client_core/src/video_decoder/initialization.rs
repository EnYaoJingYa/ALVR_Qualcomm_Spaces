use alvr_common::parking_lot::{Condvar, Mutex};

// A decoder can finish initialization successfully or fail before publishing a
// MediaCodec. Both outcomes must wake the creator, including failures that occur
// before it starts waiting.
#[derive(Default)]
pub(super) struct DecoderInitialization {
    result: Mutex<Option<Result<(), String>>>,
    changed: Condvar,
}

impl DecoderInitialization {
    pub fn complete(&self, result: Result<(), String>) {
        let mut state = self.result.lock();
        if state.is_none() {
            *state = Some(result);
            self.changed.notify_all();
        }
    }

    pub fn wait(&self) -> Result<(), String> {
        let mut state = self.result.lock();
        loop {
            if let Some(result) = state.as_ref() {
                return result.clone();
            }
            self.changed.wait(&mut state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };

    #[test]
    fn initialization_failure_wakes_the_creator() {
        let state = Arc::new(DecoderInitialization::default());
        let (sender, receiver) = mpsc::channel();
        let waiter = thread::spawn({
            let state = Arc::clone(&state);
            move || sender.send(state.wait()).unwrap()
        });
        state.complete(Err("MediaCodec configure failed".into()));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err("MediaCodec configure failed".into())
        );
        waiter.join().unwrap();
    }

    #[test]
    fn successful_initialization_is_remembered_before_waiting() {
        let state = DecoderInitialization::default();
        state.complete(Ok(()));
        // A later stream failure is reported through the frame callback; it
        // must not retroactively change the initialization result.
        state.complete(Err("decoder stopped later".into()));
        assert_eq!(state.wait(), Ok(()));
    }
}
