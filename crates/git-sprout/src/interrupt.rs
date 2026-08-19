// ABOUTME: Defers an interrupt until the worktree is complete, so Ctrl-C never leaves a
// ABOUTME: destination that git registered but nobody filled.

use std::sync::atomic::{AtomicI32, Ordering};

/// The signal that arrived, or zero. Written only from a signal handler, which may do
/// nothing but store to an atomic.
static ARRIVED: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn record(signal: libc::c_int) {
    ARRIVED.store(signal, Ordering::Relaxed);
}

/// Asks the process to note interrupts rather than die on them.
///
/// Between the moment git creates the worktree and the moment git finishes checking it out
/// there is a window where the destination exists, is registered in `git worktree list`,
/// and holds nothing. Dying inside it leaves the user a worktree that looks real and
/// reports its whole tree as deleted. Instead the request is recorded, the clone phase
/// stops at its next path, git still finishes the checkout, and the process then dies of
/// the original signal.
#[cfg(unix)]
pub fn defer() {
    let handler = record as *const () as libc::sighandler_t;
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // A signal the caller already asked to be ignored stays ignored: a shell that
        // starts a background job hands it SIGINT set to ignore, and taking it back would
        // make the tool die where git would not.
        // SAFETY: `record` only stores to an atomic, which is async-signal-safe.
        unsafe {
            if libc::signal(signal, handler) == libc::SIG_IGN {
                libc::signal(signal, libc::SIG_IGN);
            }
        }
    }
}

#[cfg(not(unix))]
pub fn defer() {}

/// Whether an interrupt is waiting to be honoured.
pub fn requested() -> bool {
    ARRIVED.load(Ordering::Relaxed) != 0
}

/// Dies of the deferred signal, if one arrived. Returns if none did.
#[cfg(unix)]
pub fn honour() {
    let signal = ARRIVED.load(Ordering::Relaxed);
    if signal == 0 {
        return;
    }
    // SAFETY: restoring the default disposition and re-raising is the documented way to
    // exit with the status the signal would have produced.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

#[cfg(not(unix))]
pub fn honour() {}
