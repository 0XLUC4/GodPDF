//! Slow core work off the UI thread, counted for the "Working…" spinner of the header.

use std::panic::{AssertUnwindSafe, catch_unwind};

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

/// Run `work` on the background executor, with the spinner shown until it is done.
/// A panic (renderer or library bug on a malformed PDF) becomes an error: on the thread pool
/// it would abort the whole app.
pub fn background<T: Send + 'static>(
    cx: &mut App,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Task<Result<T, String>> {
    cx.default_global::<Jobs>().running += 1;
    cx.refresh_windows();
    let job = cx.background_executor().spawn(async move {
        catch_unwind(AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err(crate::doc::PANIC_MESSAGE.to_owned()))
    });
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
