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
//! use advisory_lock::{AdvisoryFileLock, FileLockMode, FileLockError};
//! #
//! # std::fs::File::create("foo.txt").unwrap();
//! #
//! // Create the file
//! let mut exclusive_file = AdvisoryFileLock::new("foo.txt", FileLockMode::Exclusive)?;
//!
//! exclusive_file.lock()?;
//!
//! let mut shared_file = AdvisoryFileLock::new("foo.txt", FileLockMode::Shared)?;
//!
//! // Try to acquire the lock in non-blocking way
//! assert!(matches!(shared_file.try_lock(), Err(FileLockError::AlreadyLocked)));
//!
//! exclusive_file.unlock()?;
//!
//! shared_file.try_lock().expect("Works, because the exlusive lock was released");
//!
//! let mut shared_file_2 = AdvisoryFileLock::new("foo.txt", FileLockMode::Shared)?;
//!
//! shared_file_2.lock().expect("Should be fine to have multiple shared locks");
//!
//! // Nope, now we have to wait until all shared locks are released...
//! assert!(matches!(exclusive_file.try_lock(), Err(FileLockError::AlreadyLocked)));
//!
//! // We can unlock them explicitly and handle the potential error
//! shared_file.unlock()?;
//! // Or drop the lock, such that we `log::error!()` if it happens and discard it
//! drop(shared_file_2);
//!
//! exclusive_file.lock().expect("All other locks should have been released");
//! #
//! # std::fs::remove_file("foo.txt")?;
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! [`AdvisoryFileLock`]: struct.AdvisoryFileLock.html
//! [`RwLock`]: https://doc.rust-lang.org/stable/std/sync/struct.RwLock.html
//! [`File`]: https://doc.rust-lang.org/stable/std/fs/struct.File.html
use std::{
    fs::{File, OpenOptions},
    io,
    ops::{Deref, DerefMut},
    path::Path, fmt,
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
pub struct AdvisoryFileLock {
    /// An underlying file.
    file: File,
    /// A file lock mode, shared or exclusive.
    file_lock_mode: FileLockMode,
}

impl AdvisoryFileLock {
    /// Create a new `FileLock`.
    pub fn new<P: AsRef<Path>>(
        path: P,
        file_lock_mode: FileLockMode,
    ) -> Result<Self, FileLockError> {
        let is_exclusive = file_lock_mode == FileLockMode::Exclusive;
        let file = OpenOptions::new()
            .read(true)
            .create(is_exclusive)
            .write(is_exclusive)
            .open(path)
            .map_err(FileLockError::Io)?;

        Ok(AdvisoryFileLock {
            file,
            file_lock_mode,
        })
    }

    /// Return `true` if the advisory lock is acquired by shared mode.
    pub fn is_shared(&self) -> bool {
        self.file_lock_mode == FileLockMode::Shared
    }

    /// Return `true` if the advisory lock is acquired by exclusive mode.
    pub fn is_exclusive(&self) -> bool {
        self.file_lock_mode == FileLockMode::Exclusive
    }

    /// Acquire the advisory file lock.
    ///
    /// `lock` is blocking; it will block the current thread until it succeeds or errors.
    pub fn lock(&mut self) -> Result<(), FileLockError> {
        self.lock_impl()
    }

    /// Try to acquire the advisory file lock.
    ///
    /// `try_lock` returns immediately.
    pub fn try_lock(&mut self) -> Result<(), FileLockError> {
        self.try_lock_impl()
    }

    /// Unlock this advisory file lock.
    pub fn unlock(&mut self) -> Result<(), FileLockError> {
        self.unlock_impl()
    }
}

impl Drop for AdvisoryFileLock {
    fn drop(&mut self) {
        if let Err(err) = self.unlock() {
            log::error!(
                "Unlock_file failed during dropping: {}",
                err
            );
        }
    }
}

impl Deref for AdvisoryFileLock {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl DerefMut for AdvisoryFileLock {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.file
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn simple_shared_lock() {
        let mut test_file = temp_dir();
        test_file.push("shared_lock");
        File::create(&test_file).unwrap();
        {
            let mut f1 = AdvisoryFileLock::new(&test_file, FileLockMode::Shared).unwrap();
            f1.lock().unwrap();
            let mut f2 = AdvisoryFileLock::new(&test_file, FileLockMode::Shared).unwrap();
            f2.lock().unwrap();
        }
        std::fs::remove_file(&test_file).unwrap();
    }

    #[test]
    fn simple_exclusive_lock() {
        let mut test_file = temp_dir();
        test_file.push("exclusive_lock");
        {
            let mut f1 = AdvisoryFileLock::new(&test_file, FileLockMode::Exclusive).unwrap();
            f1.lock().unwrap();
            let f2 = AdvisoryFileLock::new(&test_file, FileLockMode::Exclusive)
                .unwrap()
                .try_lock();
            assert!(f2.is_err());
        }
        std::fs::remove_file(&test_file).unwrap();
    }

    #[test]
    fn simple_shared_exclusive_lock() {
        let mut test_file = temp_dir();
        test_file.push("shared_exclusive_lock");
        File::create(&test_file).unwrap();
        {
            let mut f1 = AdvisoryFileLock::new(&test_file, FileLockMode::Shared).unwrap();
            f1.lock().unwrap();
            let mut f2 = AdvisoryFileLock::new(&test_file, FileLockMode::Exclusive).unwrap();
            assert!(matches!(f2.try_lock(), Err(FileLockError::AlreadyLocked)));
        }
        std::fs::remove_file(&test_file).unwrap();
    }

    #[test]
    fn simple_exclusive_shared_lock() {
        let mut test_file = temp_dir();
        test_file.push("exclusive_shared_lock");
        {
            let mut f1 = AdvisoryFileLock::new(&test_file, FileLockMode::Exclusive).unwrap();
            f1.lock().unwrap();
            let mut f2 = AdvisoryFileLock::new(&test_file, FileLockMode::Exclusive).unwrap();
            assert!(f2.try_lock().is_err());
        }
        std::fs::remove_file(&test_file).unwrap();
    }
}
