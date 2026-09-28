use std::{
    io::{self, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::Duration,
};

/// Terminal rendering for an operation's progress. Query evaluation can use the
/// same handle when it has progress to report.
pub(crate) struct Reporter {
    animated: bool,
}

impl Reporter {
    pub fn new(animated: bool) -> Self {
        Self { animated }
    }

    pub fn start(&self, label: impl Into<String>) -> Handle {
        let (sender, receiver) = mpsc::channel();
        let animated = self.animated;
        let initial = Update::new(label, None, None);
        let worker = thread::spawn(move || render(receiver, initial, animated));
        Handle {
            sender,
            worker: Some(worker),
        }
    }
}

pub(crate) struct Handle {
    sender: Sender<Message>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Handle {
    // Query evaluation will call this when it can report stages or counts.
    #[allow(dead_code)]
    pub fn update(&self, label: impl Into<String>, completed: Option<u64>, total: Option<u64>) {
        let _ = self
            .sender
            .send(Message::Update(Update::new(label, completed, total)));
    }

    pub fn finish(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = self.sender.send(Message::Stop);
            let _ = worker.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Update {
    label: String,
    completed: Option<u64>,
    total: Option<u64>,
}

impl Update {
    fn new(label: impl Into<String>, completed: Option<u64>, total: Option<u64>) -> Self {
        Self {
            label: label.into(),
            completed,
            total,
        }
    }

    fn text(&self) -> String {
        match (self.completed, self.total) {
            (Some(done), Some(total)) if total > 0 => {
                format!("{} ({}/{})", self.label, done.min(total), total)
            }
            (Some(done), _) => format!("{} ({done})", self.label),
            _ => self.label.clone(),
        }
    }
}

enum Message {
    #[allow(dead_code)]
    Update(Update),
    Stop,
}

fn render(receiver: Receiver<Message>, mut state: Update, animated: bool) {
    let mut stderr = io::stderr().lock();
    let _ = render_to(receiver, &mut state, animated, &mut stderr);
}

fn render_to(
    receiver: Receiver<Message>,
    state: &mut Update,
    animated: bool,
    writer: &mut impl Write,
) -> io::Result<()> {
    const FRAMES: &[&str] = &["◐", "◓", "◑", "◒"];
    let mut frame = 0;
    if animated {
        write!(writer, "\r\x1b[2K{} {}", FRAMES[frame], state.text())?;
    } else {
        writeln!(writer, "{}...", state.text())?;
    }
    writer.flush()?;
    loop {
        let message = if animated {
            receiver.recv_timeout(Duration::from_millis(100))
        } else {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match message {
            Ok(Message::Update(update)) => {
                *state = update;
                if !animated {
                    writeln!(writer, "{}...", state.text())?;
                }
            }
            Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                frame = (frame + 1) % FRAMES.len();
            }
        }
        if animated {
            write!(writer, "\r\x1b[2K{} {}", FRAMES[frame], state.text())?;
        }
        writer.flush()?;
    }
    if animated {
        write!(writer, "\r\x1b[2K")?;
        writer.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_text_supports_indeterminate_and_fractional_updates() {
        assert_eq!(
            Update::new("Compiling request", None, None).text(),
            "Compiling request"
        );
        assert_eq!(
            Update::new("Evaluating query", Some(3), Some(8)).text(),
            "Evaluating query (3/8)"
        );
        assert_eq!(
            Update::new("Evaluating query", Some(12), Some(8)).text(),
            "Evaluating query (8/8)"
        );
        assert_eq!(
            Update::new("Scanning rows", Some(12), None).text(),
            "Scanning rows (12)"
        );
    }

    #[test]
    fn progress_clears_on_finish_and_has_escape_free_static_fallback() {
        for animated in [false, true] {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(Message::Update(Update::new(
                    "Evaluating query",
                    Some(1),
                    Some(2),
                )))
                .unwrap();
            sender.send(Message::Stop).unwrap();
            let mut output = Vec::new();
            render_to(
                receiver,
                &mut Update::new("Compiling request", None, None),
                animated,
                &mut output,
            )
            .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(output.contains("Compiling request"));
            assert!(output.contains("Evaluating query (1/2)"));
            if animated {
                assert!(output.ends_with("\r\x1b[2K"));
            } else {
                assert!(!output.contains('\x1b'));
                assert!(output.ends_with("Evaluating query (1/2)...\n"));
            }
        }
        let (sender, receiver) = mpsc::channel();
        drop(sender); // A dropped operation still clears the animated line.
        let mut output = Vec::new();
        render_to(
            receiver,
            &mut Update::new("Compiling request", None, None),
            true,
            &mut output,
        )
        .unwrap();
        assert!(output.ends_with(b"\r\x1b[2K"));
    }
}
