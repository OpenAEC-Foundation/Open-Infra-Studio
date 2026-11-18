//! Background work for the window: network calls never block the UI thread.

use crate::lang::tr;
use std::sync::{Arc, Mutex};

type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

/// A running job with a live status line.
pub struct Job<T> {
    result: Slot<T>,
    status: Arc<Mutex<String>>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(label: &str, f: impl FnOnce(&dyn Fn(String)) -> Result<T, String> + Send + 'static) -> Self {
        let document = crate::bridge::target().map(|target| (target, crate::bridge::site()));
        let result: Slot<T> = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(label.to_string()));
        let (r, s) = (result.clone(), status.clone());
        std::thread::spawn(move || {
            let progress = |msg: String| {
                *s.lock().unwrap() = msg;
                super::repaint();
            };
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match document {
                Some((target, site)) => crate::bridge::with_command_context(target, site, None, || f(&progress)),
                None => f(&progress),
            }))
                .unwrap_or_else(|_| Err(tr("interne fout in de import", "internal import error").into()));
            *r.lock().unwrap() = Some(out);
            super::repaint();
        });
        Self { result, status }
    }

    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    /// The result, once; `None` while still running.
    pub fn take(&self) -> Option<Result<T, String>> {
        self.result.lock().unwrap().take()
    }
}

/// Poll an optional job slot: returns the result when it finished.
pub fn poll<T: Send + 'static>(job: &mut Option<Job<T>>) -> Option<Result<T, String>> {
    let done = job.as_ref()?.take();
    if done.is_some() {
        *job = None;
    }
    done
}
