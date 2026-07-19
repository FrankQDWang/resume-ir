use std::io::{self, Read};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};

pub(super) enum BoundedPipeReadError {
    TooLarge,
    Io(io::Error),
    ReaderPanicked,
}

pub(super) struct BoundedPipeReader {
    receiver: Receiver<std::result::Result<Vec<u8>, BoundedPipeReadError>>,
    handle: JoinHandle<()>,
    outcome: Option<std::result::Result<Vec<u8>, BoundedPipeReadError>>,
}

impl BoundedPipeReader {
    pub(super) fn spawn<R>(mut reader: R, max_bytes: usize) -> Self
    where
        R: Read + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut output = Vec::new();
            let mut buffer = [0_u8; 8192];
            let outcome = loop {
                let read = match reader.read(&mut buffer) {
                    Ok(read) => read,
                    Err(error) => break Err(BoundedPipeReadError::Io(error)),
                };
                if read == 0 {
                    break Ok(output);
                }
                if output.len().saturating_add(read) > max_bytes {
                    break Err(BoundedPipeReadError::TooLarge);
                }
                output.extend_from_slice(&buffer[..read]);
            };
            let _ = sender.send(outcome);
        });
        Self {
            receiver,
            handle,
            outcome: None,
        }
    }

    pub(super) fn limit_exceeded(&mut self) -> bool {
        self.poll();
        matches!(
            self.outcome.as_ref(),
            Some(Err(BoundedPipeReadError::TooLarge))
        )
    }

    pub(super) fn finish(mut self) -> std::result::Result<Vec<u8>, BoundedPipeReadError> {
        let outcome = self.outcome.take().or_else(|| self.receiver.recv().ok());
        if self.handle.join().is_err() {
            return Err(BoundedPipeReadError::ReaderPanicked);
        }
        outcome.unwrap_or(Err(BoundedPipeReadError::ReaderPanicked))
    }

    fn poll(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        match self.receiver.try_recv() {
            Ok(outcome) => self.outcome = Some(outcome),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.outcome = Some(Err(BoundedPipeReadError::ReaderPanicked));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn limit_failure_is_observable_before_parent_joins_the_reader() {
        let mut reader = BoundedPipeReader::spawn(&b"oversized"[..], /*max_bytes*/ 4);
        let deadline = Instant::now() + Duration::from_secs(1);

        while !reader.limit_exceeded() && Instant::now() < deadline {
            thread::yield_now();
        }

        assert!(reader.limit_exceeded());
        assert!(matches!(
            reader.finish(),
            Err(BoundedPipeReadError::TooLarge)
        ));
    }
}
