//! Single-instance guard: an exclusive OS lock on `<data dir>/daemon.lock`.
//! The OS drops the lock when the process dies, so a lock file left behind
//! by a crash is stale by construction and is simply re-locked.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub struct InstanceLock {
    _file: File,
}

#[derive(Debug)]
pub enum LockError {
    Held(String),
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held(pid) => write!(
                f,
                "another xdm-daemon is already running for this data directory{}; stop it first",
                if pid.is_empty() { String::new() } else { format!(" (pid {pid})") }
            ),
            LockError::Io(e) => write!(f, "cannot take the daemon lock: {e}"),
        }
    }
}

#[cfg(unix)]
fn try_lock(file: &File) -> io::Result<bool> {
    use std::os::unix::io::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) { Ok(false) } else { Err(error) }
}

#[cfg(windows)]
fn try_lock(file: &File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx};
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as HANDLE,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) { Ok(false) } else { Err(error) }
}

pub fn acquire(dir: &Path) -> Result<InstanceLock, LockError> {
    let path = dir.join("daemon.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(LockError::Io)?;
    if !try_lock(&file).map_err(LockError::Io)? {
        let pid = std::fs::read_to_string(&path).unwrap_or_default().trim().to_owned();
        return Err(LockError::Held(pid));
    }
    // Informational only; the lock itself is the source of truth.
    let _ = file.set_len(0);
    let _ = write!(file, "{}", std::process::id());
    let _ = file.flush();
    Ok(InstanceLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_fails_and_stale_file_is_reusable() {
        let dir = std::env::temp_dir().join(format!("xdm-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = acquire(&dir).unwrap();
        assert!(matches!(acquire(&dir), Err(LockError::Held(_))));
        drop(first);
        // The file remains (stale after a crash) but the lock is free again.
        assert!(dir.join("daemon.lock").exists());
        acquire(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
