//! Slow core work off the UI thread, counted for the "Working…" spinner of the header.

use std::future::Future;

use gpui::{App, Global, Task};

#[derive(Default)]
struct Jobs {
    running: usize,
}

impl Global for Jobs {}

/// A background job is running.
pub fn busy(cx: &App) -> bool {
    cx.try_global::<Jobs>().is_some_and(|jobs| jobs.running > 0)
}

/// `cx.background_executor().spawn(work)`, with the spinner shown until it is done.
pub fn background<R: Send + 'static>(
    cx: &mut App,
    work: impl Future<Output = R> + Send + 'static,
) -> Task<R> {
    cx.default_global::<Jobs>().running += 1;
    cx.refresh_windows();
    let job = cx.background_executor().spawn(work);
    cx.spawn(async move |cx| {
        let result = job.await;
        cx.update(|cx| {
            let jobs = cx.default_global::<Jobs>();
            jobs.running = jobs.running.saturating_sub(1);
            cx.refresh_windows();
        });
        result
    })
}
