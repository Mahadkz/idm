//! Termination signals: SIGTERM/SIGINT/SIGHUP on Unix, console control
//! events on Windows. The handler thread drives `Service::shutdown`.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Must run before any thread is spawned so every thread inherits the mask
/// and only the waiter below receives the signals.
#[cfg(unix)]
pub fn block_signals() {
    unsafe {
        let mut set = std::mem::zeroed::<libc::sigset_t>();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, signal);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

#[cfg(not(unix))]
pub fn block_signals() {}

/// Blocks until a termination signal arrives.
#[cfg(unix)]
pub fn wait_for_signal() {
    unsafe {
        let mut set = std::mem::zeroed::<libc::sigset_t>();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, signal);
        }
        let mut received = 0;
        libc::sigwait(&set, &mut received);
    }
}

#[cfg(windows)]
static CTRL: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
unsafe extern "system" fn ctrl_handler(_event: u32) -> i32 {
    CTRL.store(true, Ordering::SeqCst);
    // Keep the process alive until shutdown finishes; it exits itself.
    std::thread::sleep(std::time::Duration::from_secs(30));
    1
}

#[cfg(windows)]
pub fn wait_for_signal() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), 1) };
    while !CTRL.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
