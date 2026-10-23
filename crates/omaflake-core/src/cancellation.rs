// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Cancellation.swift.
//!
//! Cancels long work (image builds): `cancel()` terminates the processes `Shell` is running right now and runs the
//! registered handlers (e.g. forcing a build VM off); the work checks `check()` between its steps.
//!
//! Swift passed the token as a task-local value (`Cancellation.$current.withValue(token) { … }`) so `Shell` found it
//! without being handed through every call. Rust has no task locals: long-running functions take a `&Cancellation`
//! parameter and hand it to `Shell` (`RunOptions::cancel`). A token is cheap to clone (an `Arc` inside); clones
//! cancel together. `Cancellation::new()` is a token nobody cancels – pass it where the caller has none.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{Error, Result};
use crate::spawn::Pid;

type Handler = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct State {
    cancelled: bool,
    processes: Vec<Pid>,
    handlers: Vec<Handler>,
}

/// A cancellation token (see the module docs)
#[derive(Clone, Default)]
pub struct Cancellation {
    inner: Arc<Mutex<State>>,
}

impl std::fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cancellation").field("cancelled", &self.is_cancelled()).finish()
    }
}

impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // A handler that panicked must not make the token unusable
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_cancelled(&self) -> bool {
        self.state().cancelled
    }

    /// Stops the running processes (SIGTERM, like Swift's `Process.terminate()`) and calls the handlers; later steps
    /// see `is_cancelled`. A second call does nothing.
    pub fn cancel(&self) {
        let (procs, handlers) = {
            let mut s = self.state();
            if s.cancelled {
                return;
            }
            s.cancelled = true;
            (s.processes.clone(), std::mem::take(&mut s.handlers))
        };
        for pid in procs {
            terminate(pid);
        }
        for h in handlers {
            h();
        }
    }

    /// Errs with "Cancelled" (code `cancelled`) when cancelled – call it between steps.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::cancelled())
        } else {
            Ok(())
        }
    }

    /// Runs `handler` on cancel (right away if already cancelled).
    pub fn on_cancel(&self, handler: impl FnOnce() + Send + 'static) {
        let now = {
            let mut s = self.state();
            if !s.cancelled {
                s.handlers.push(Box::new(handler));
                return;
            }
            true
        };
        if now {
            handler();
        }
    }

    /// A running child process that `cancel` terminates. It must not have been reaped yet – `Shell` waits with
    /// `WNOWAIT`, untracks, and only then reaps, so a PID is never signalled after it could have been reused.
    pub(crate) fn track(&self, pid: Pid) {
        {
            let mut s = self.state();
            if !s.cancelled {
                s.processes.push(pid);
                return;
            }
        }
        terminate(pid);
    }

    pub(crate) fn untrack(&self, pid: Pid) {
        self.state().processes.retain(|p| *p != pid);
    }
}

fn terminate(pid: Pid) {
    // SAFETY: plain libc call; the PID belongs to a child that has not been reaped (see `track`)
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn handlers_run_once_and_late_ones_at_once() {
        let t = Cancellation::new();
        let n = Arc::new(AtomicUsize::new(0));
        let a = n.clone();
        t.on_cancel(move || {
            a.fetch_add(1, Ordering::SeqCst);
        });
        assert!(t.check().is_ok());
        let clone = t.clone();
        clone.cancel();
        clone.cancel();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(t.is_cancelled());
        assert!(t.check().unwrap_err().is_cancelled());
        let b = n.clone();
        t.on_cancel(move || {
            b.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }
}
