use std::io::Error;
use std::os::unix::io::{AsRawFd, RawFd};

use crate::{AdvisoryFileLock, FileLockError, FileLockMode};

impl AdvisoryFileLock {
    pub(super) fn lock_impl(&mut self) -> Result<(), FileLockError> {
        lock_file(self.file.as_raw_fd(), self.file_lock_mode, false)
    }

    pub(super) fn try_lock_impl(&mut self) -> Result<(), FileLockError> {
        lock_file(self.file.as_raw_fd(), self.file_lock_mode, true)
    }

    pub(super) fn unlock_impl(&mut self) -> Result<(), FileLockError> {
        unlock_file(self.file.as_raw_fd())
    }
}

fn lock_file(
    raw_fd: RawFd,
    file_lock_mode: FileLockMode,
    immediate: bool,
) -> Result<(), FileLockError> {
    let mut flags = match file_lock_mode {
        FileLockMode::Shared => libc::LOCK_SH,
        FileLockMode::Exclusive => libc::LOCK_EX,
    };
    if immediate {
        flags |= libc::LOCK_NB;
    }

    let result = unsafe { libc::flock(raw_fd, flags) };
    if result != 0 {
        let last_os_error = Error::last_os_error();
        return Err(match last_os_error.raw_os_error() {
            Some(code) if code == libc::EWOULDBLOCK => FileLockError::AlreadyLocked,
            _ => FileLockError::IOError(last_os_error),
        });
    }

    Ok(())
}

fn unlock_file(raw_fd: RawFd) -> Result<(), FileLockError> {
    let result = unsafe { libc::flock(raw_fd, libc::LOCK_UN) };
    if result == 0 {
        Ok(())
    } else {
        Err(FileLockError::IOError(Error::last_os_error()))
    }
}
