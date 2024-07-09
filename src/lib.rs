//! Advisory lock provides simple and convenient API for using file locks.
//!
//! These are called advisory because they don't prevent other processes from
//! accessing the files directly, bypassing the locks.
//! However, if multiple processes agree on acquiring file locks, they should
//! work as expected.
//!
//! The main entity of the crate is [`AdvisoryFileLock`] which is effectively
//! a [`RwLock`] but for [`File`].
//!
//! Example:
//! ```
//! use std::fs::File;
//! use advisory_lock::{AdvisoryFileLock, FileLockMode, FileLockError};
//! #
//! #
//! // Create the file and obtain its exclusive advisory lock
//! let exclusive_file = File::create("foo.txt").unwrap();
//! exclusive_file.lock(FileLockMode::Exclusive)?;
//!
//! let shared_file = File::open("foo.txt")?;
//!
//! // Try to acquire the lock in non-blocking way
//! assert!(matches!(shared_file.try_lock(FileLockMode::Shared), Err(FileLockError::AlreadyLocked)));
//!
//! exclusive_file.unlock()?;
//!
//! shared_file.try_lock(FileLockMode::Shared).expect("Works, because the exclusive lock was released");
//!
//! let shared_file_2 = File::open("foo.txt")?;
//!
//! shared_file_2.lock(FileLockMode::Shared).expect("Should be fine to have multiple shared locks");
//!
//! // Nope, now we have to wait until all shared locks are released...
//! assert!(matches!(exclusive_file.try_lock(FileLockMode::Exclusive), Err(FileLockError::AlreadyLocked)));
//!
//! // We can unlock them explicitly and handle the potential error
//! shared_file.unlock()?;
//! // Or drop the lock, such that we `log::error!()` if it happens and discard it
//! drop(shared_file_2);
//!
//! exclusive_file.lock(FileLockMode::Exclusive).expect("All other locks should have been released");
//! #
//! # std::fs::remove_file("foo.txt")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! [`AdvisoryFileLock`]: struct.AdvisoryFileLock.html
//! [`RwLock`]: https://doc.rust-lang.org/stable/std/sync/struct.RwLock.html
//! [`File`]: https://doc.rust-lang.org/stable/std/fs/struct.File.html
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, Write},
    mem::ManuallyDrop,
    ops::Deref,
    path::Path,
    ptr::addr_of,
};

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

/// An enumeration of possible errors which can occur while trying to acquire a lock.
#[derive(Debug)]
pub enum FileLockError {
    /// The file is already locked by other process.
    AlreadyLocked,
    /// The error occurred during I/O operations.
    Io(io::Error),
}

impl fmt::Display for FileLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileLockError::AlreadyLocked => f.write_str("the file is already locked"),
            FileLockError::Io(err) => write!(f, "I/O error: {}", err),
        }
    }
}

impl std::error::Error for FileLockError {}

/// An enumeration of types which represents how to acquire an advisory lock.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum FileLockMode {
    /// Obtain an exclusive file lock.
    Exclusive,
    /// Obtain a shared file lock.
    Shared,
}

/// An advisory lock for files.
///
/// An advisory lock provides a mutual-exclusion mechanism among processes which explicitly
/// acquires and releases the lock. Processes that are not aware of the lock will ignore it.
///
/// `AdvisoryFileLock` provides following features:
/// - Blocking or non-blocking operations.
/// - Shared or exclusive modes.
/// - All operations are thread-safe.
///
/// ## Notes
///
/// `AdvisoryFileLock` has following limitations:
/// - Locks are allowed only on files, but not directories.
pub trait AdvisoryFileLock {
    /// Acquire the advisory file lock.
    ///
    /// `lock` is blocking; it will block the current thread until it succeeds or errors.
    fn lock(&self, file_lock_mode: FileLockMode) -> Result<(), FileLockError>;
    /// Try to acquire the advisory file lock.
    ///
    /// `try_lock` returns immediately.
    fn try_lock(&self, file_lock_mode: FileLockMode) -> Result<(), FileLockError>;
    /// Unlock this advisory file lock.
    fn unlock(&self) -> Result<(), FileLockError>;
}

/// A guard to a locked file that will automatically unlock it's underlying file when dropped.
///
/// ## Example
/// ```rust
/// use std::fs::File;
/// use advisory_lock::{LockGuard, FileLockMode, OpenOptionsExt};
///
/// // Opens the file with an exclusive lock
/// let mut file: LockGuard<File> = File::options()
///    .write(true)
///    .truncate(true)
///    .open_locked("test_file.txt", FileLockMode::Exclusive)
///    .unwrap();
///
/// file.write(b"Hello world!").unwrap();
/// // When the `file` variable drops, it's underlying file will be unlocked
/// ```
#[derive(Debug)]
pub struct LockGuard<T: ?Sized + AdvisoryFileLock> {
    inner: T,
}

impl<T: AdvisoryFileLock> LockGuard<T> {
    /// Acquire the advisory file lock.
    ///
    /// `lock` is blocking; it will block the current thread until it succeeds or errors.
    pub fn lock(file: T, file_lock_mode: FileLockMode) -> std::io::Result<Self> {
        match file.lock(file_lock_mode) {
            Ok(()) => Ok(Self { inner: file }),
            Err(FileLockError::AlreadyLocked) => Err(std::io::ErrorKind::PermissionDenied.into()),
            Err(FileLockError::Io(e)) => Err(e),
        }
    }

    /// Try to acquire the advisory file lock.
    ///
    /// `try_lock` returns immediately.
    pub fn try_lock(file: T, file_lock_mode: FileLockMode) -> std::io::Result<Option<Self>> {
        match file.try_lock(file_lock_mode) {
            Ok(()) => Ok(Some(Self { inner: file })),
            Err(FileLockError::AlreadyLocked) => Ok(None),
            Err(FileLockError::Io(e)) => Err(e),
        }
    }

    /// Returns the underlying file handle, unlocking the file in the process.
    pub fn try_unlock(self) -> Result<T, (FileLockError, Self)> {
        if let Err(e) = self.inner.unlock() {
            return Err((e, self));
        }

        let this = ManuallyDrop::new(self);
        return Ok(unsafe { std::ptr::read(addr_of!(this.inner)) });
    }
}

impl<T: ?Sized + AdvisoryFileLock + Read> Read for LockGuard<T> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<T: ?Sized + AdvisoryFileLock + Write> Write for LockGuard<T> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: ?Sized + AdvisoryFileLock + Seek> Seek for LockGuard<T> {
    #[inline]
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl<T: ?Sized + AdvisoryFileLock> Deref for LockGuard<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: ?Sized + AdvisoryFileLock> Drop for LockGuard<T> {
    #[inline]
    fn drop(&mut self) {
        let _ = self.inner.unlock();
    }
}

/// Extended file locking funtionality for [`File`]
pub trait FileExt {
    /// Opens a file and acquires the advisory file lock.
    ///
    /// See [`open`](std::fs::File::open) and [`lock`](AdvisoryFileLock::lock).
    fn open_locked<P>(path: P, file_lock_mode: FileLockMode) -> std::io::Result<LockGuard<File>>
    where
        P: AsRef<Path>;

    /// Opens a file and tries to acquire the advisory file lock.
    ///
    /// See [`open`](std::fs::File::open) and [`try_lock`](AdvisoryFileLock::try_lock).
    fn try_open_locked<P>(
        path: P,
        file_lock_mode: FileLockMode,
    ) -> std::io::Result<Option<LockGuard<File>>>
    where
        P: AsRef<Path>;
}

impl FileExt for File {
    fn open_locked<P>(path: P, file_lock_mode: FileLockMode) -> std::io::Result<LockGuard<File>>
    where
        P: AsRef<Path>,
    {
        return LockGuard::lock(File::open(path)?, file_lock_mode);
    }

    fn try_open_locked<P>(
        path: P,
        file_lock_mode: FileLockMode,
    ) -> std::io::Result<Option<LockGuard<File>>>
    where
        P: AsRef<Path>,
    {
        return LockGuard::try_lock(File::open(path)?, file_lock_mode);
    }
}

/// Extended file locking funtionality for [`OpenOptions`]
pub trait OpenOptionsExt {
    /// Opens a file and acquires the advisory file lock.
    ///
    /// See [`open`](std::fs::File::open) and [`lock`](AdvisoryFileLock::lock).
    fn open_locked<P>(
        &self,
        path: P,
        file_lock_mode: FileLockMode,
    ) -> std::io::Result<LockGuard<File>>
    where
        P: AsRef<Path>;

    /// Opens a file and tries to acquire the advisory file lock.
    ///
    /// See [`open`](std::fs::File::open) and [`try_lock`](AdvisoryFileLock::try_lock).
    fn try_open_locked<P>(
        &self,
        path: P,
        file_lock_mode: FileLockMode,
    ) -> std::io::Result<Option<LockGuard<File>>>
    where
        P: AsRef<Path>;
}

impl OpenOptionsExt for OpenOptions {
    fn open_locked<P>(
        &self,
        path: P,
        file_lock_mode: FileLockMode,
    ) -> std::io::Result<crate::LockGuard<File>>
    where
        P: AsRef<std::path::Path>,
    {
        return LockGuard::lock(self.open(path)?, file_lock_mode);
    }

    fn try_open_locked<P>(
        &self,
        path: P,
        file_lock_mode: FileLockMode,
    ) -> std::io::Result<Option<crate::LockGuard<File>>>
    where
        P: AsRef<std::path::Path>,
    {
        return LockGuard::try_lock(self.open(path)?, file_lock_mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::fs::File;

    #[test]
    fn simple_shared_lock() {
        let mut test_file = temp_dir();
        test_file.push("shared_lock");
        File::create(&test_file).unwrap();
        {
            let f1 = File::open(&test_file).unwrap();
            f1.lock(FileLockMode::Shared).unwrap();
            let f2 = File::open(&test_file).unwrap();
            f2.lock(FileLockMode::Shared).unwrap();
        }
        std::fs::remove_file(&test_file).unwrap();
    }

    #[test]
    fn simple_exclusive_lock() {
        let mut test_file = temp_dir();
        test_file.push("exclusive_lock");
        File::create(&test_file).unwrap();
        {
            let f1 = File::open(&test_file).unwrap();
            f1.lock(FileLockMode::Exclusive).unwrap();
            let f2 = File::open(&test_file).unwrap();
            assert!(f2.try_lock(FileLockMode::Exclusive).is_err());
        }
        std::fs::remove_file(&test_file).unwrap();
    }

    #[test]
    fn simple_shared_exclusive_lock() {
        let mut test_file = temp_dir();
        test_file.push("shared_exclusive_lock");
        File::create(&test_file).unwrap();
        {
            let f1 = File::open(&test_file).unwrap();
            f1.lock(FileLockMode::Shared).unwrap();
            let f2 = File::open(&test_file).unwrap();
            assert!(matches!(
                f2.try_lock(FileLockMode::Exclusive),
                Err(FileLockError::AlreadyLocked)
            ));
        }
        std::fs::remove_file(&test_file).unwrap();
    }

    #[test]
    fn simple_exclusive_shared_lock() {
        let mut test_file = temp_dir();
        test_file.push("exclusive_shared_lock");
        File::create(&test_file).unwrap();
        {
            let f1 = File::open(&test_file).unwrap();
            f1.lock(FileLockMode::Exclusive).unwrap();
            let f2 = File::open(&test_file).unwrap();
            assert!(f2.try_lock(FileLockMode::Shared).is_err());
        }
        std::fs::remove_file(&test_file).unwrap();
    }

    #[test]
    fn simple_lock_guard() {
        let mut test_file = temp_dir();
        test_file.push("lock_guard");
        File::create(&test_file).unwrap();
        {
            let _f1 = File::open_locked(&test_file, FileLockMode::Exclusive).unwrap();
            assert!(File::try_open_locked(&test_file, FileLockMode::Shared)
                .unwrap()
                .is_none());
        }
        std::fs::remove_file(&test_file).unwrap();
    }
}
