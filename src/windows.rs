use std::io;
use std::os::windows::io::{AsRawHandle, RawHandle};

use winapi::{
    shared::{
        minwindef::TRUE,
        ntdef::NULL,
        winerror::{ERROR_LOCKED, ERROR_NOT_LOCKED},
    },
    um::{
        errhandlingapi::GetLastError,
        fileapi::{LockFileEx, UnlockFileEx},
        minwinbase::{
            OVERLAPPED_u, OVERLAPPED_u_s, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
            OVERLAPPED,
        },
    },
};

use crate::{AdvisoryFileLock, FileLockError, FileLockMode};

impl AdvisoryFileLock {
    pub(super) fn lock_impl(&mut self) -> Result<(), FileLockError> {
        lock_file(self.file.as_raw_handle(), self.file_lock_mode, false)
    }

    pub(super) fn try_lock_impl(&mut self) -> Result<(), FileLockError> {
        lock_file(self.file.as_raw_handle(), self.file_lock_mode, true)
    }

    pub(super) fn unlock_impl(&mut self) -> Result<(), FileLockError> {
        unlock_file(self.file.as_raw_handle())
    }
}

fn create_overlapped() -> OVERLAPPED {
    let overlapped = unsafe {
        let mut overlapped = std::mem::zeroed::<OVERLAPPED_u>();
        *overlapped.s_mut() = OVERLAPPED_u_s {
            Offset: u32::MAX,
            OffsetHigh: u32::MAX,
        };
        overlapped
    };

    OVERLAPPED {
        Internal: usize::MAX,
        InternalHigh: usize::MAX,
        u: overlapped,
        hEvent: NULL,
    }
}

fn lock_file(
    raw_handle: RawHandle,
    file_lock_mode: FileLockMode,
    immediate: bool,
) -> Result<(), FileLockError> {
    let mut overlapped = create_overlapped();

    let mut flags = 0;
    if file_lock_mode == FileLockMode::Exclusive {
        flags |= LOCKFILE_EXCLUSIVE_LOCK;
    }
    if immediate {
        flags |= LOCKFILE_FAIL_IMMEDIATELY;
    }

    let result = unsafe {
        LockFileEx(
            raw_handle as *mut winapi::ctypes::c_void,
            flags,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if result != TRUE {
        return match unsafe { GetLastError() } {
            ERROR_LOCKED => Err(FileLockError::AlreadyLocked),
            raw_error => Err(FileLockError::Io(io::Error::from_raw_os_error(
                raw_error as i32,
            ))),
        };
    }

    Ok(())
}

fn unlock_file(raw_handle: RawHandle) -> Result<(), FileLockError> {
    let mut overlapped = create_overlapped();

    let result = unsafe {
        UnlockFileEx(
            raw_handle as *mut winapi::ctypes::c_void,
            0,
            1,
            0,
            &mut overlapped,
        )
    };

    if result == TRUE {
        Ok(())
    } else {
        let raw_error = unsafe { GetLastError() };
        if raw_error == ERROR_NOT_LOCKED {
            Ok(())
        } else {
            Err(FileLockError::Io(io::Error::from_raw_os_error(
                raw_error as i32,
            )))
        }
    }
}
