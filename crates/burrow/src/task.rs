//! Running closures on CEF's UI thread.

use std::sync::{Arc, Mutex};

use cef::*;

type Closure = Arc<Mutex<Option<Box<dyn Send + FnOnce()>>>>;

wrap_task! {
    struct OnceTask {
        f: Closure,
    }
    impl Task {
        fn execute(&self) {
            if let Some(f) = self.f.lock().ok().and_then(|mut f| f.take()) {
                f();
            }
        }
    }
}

/// Runs `f` on the UI thread later, after the current callback has returned.
pub fn post_ui(f: impl Send + FnOnce() + 'static) {
    let mut task = OnceTask::new(Arc::new(Mutex::new(Some(Box::new(f)))));
    post_task(ThreadId::UI, Some(&mut task));
}

pub fn post_ui_delayed(delay_ms: i64, f: impl Send + FnOnce() + 'static) {
    let mut task = OnceTask::new(Arc::new(Mutex::new(Some(Box::new(f)))));
    post_delayed_task(ThreadId::UI, Some(&mut task), delay_ms);
}
