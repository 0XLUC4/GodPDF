//! Slow core work (search, text extraction, split) off the UI thread.
//!
//! Native: each job runs on its own thread and reports through a channel, waking the UI.
//! Web (no threads): the job runs inline and its result is delivered next frame the same way.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::Context;
use godpdf_core::SearchHit;

/// A finished job.
pub enum Done {
    Search {
        tab: u64,
        generation: u64,
        query: String,
        result: Result<Vec<SearchHit>, String>,
    },
    Text {
        title: String,
        result: Result<Vec<String>, String>,
    },
    Split {
        tab: u64,
        base: String,
        to_disk: bool,
        /// Dialog fields, to reopen it with the error.
        every: bool,
        n: usize,
        starts: String,
        result: Result<Vec<Vec<u8>>, String>,
    },
    /// Every comment of a document revision, for the comments panel.
    Comments {
        tab: u64,
        generation: u64,
        result: Result<Vec<crate::comments::Entry>, String>,
    },
    /// The job panicked or could not start.
    Failed(String),
}

pub struct Jobs {
    sender: Sender<Done>,
    receiver: Receiver<Done>,
    running: usize,
}

impl Default for Jobs {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            running: 0,
        }
    }
}

impl Jobs {
    pub fn busy(&self) -> bool {
        self.running > 0
    }

    /// Run `work` in the background; its [`Done`] shows up in [`Jobs::drain`].
    pub fn spawn(&mut self, ctx: &Context, work: impl FnOnce() -> Done + Send + 'static) {
        self.running += 1;
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        let job = move || {
            let done = catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| {
                Done::Failed("Internal error while processing this PDF".to_owned())
            });
            // The receiver lives as long as the app; nothing to do if it is gone.
            let _ = sender.send(done);
            ctx.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(err) = std::thread::Builder::new()
            .name("job".to_owned())
            .stack_size(16 << 20)
            .spawn(job)
        {
            let _ = self
                .sender
                .send(Done::Failed(format!("Cannot start background work: {err}")));
        }
        #[cfg(target_arch = "wasm32")]
        job();
    }

    pub fn drain(&mut self) -> Vec<Done> {
        let done: Vec<Done> = self.receiver.try_iter().collect();
        self.running = self.running.saturating_sub(done.len());
        done
    }
}
