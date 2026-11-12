//! Poison-tolerant lock acquisition.
//!
//! A std `Mutex`/`RwLock` is poisoned when a thread panics while holding it,
//! and every later `.lock_unpoisoned()` then panics too — taking the UI thread
//! (and the whole app) down with it. Recover the guard instead of propagating
//! the panic.

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub trait MutexExt<T: ?Sized> {
    /// `lock()`, ignoring poisoning.
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub trait RwLockExt<T: ?Sized> {
    /// `read()`, ignoring poisoning.
    fn read_unpoisoned(&self) -> RwLockReadGuard<'_, T>;
    /// `write()`, ignoring poisoning.
    fn write_unpoisoned(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    fn read_unpoisoned(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_unpoisoned(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|e| e.into_inner())
    }
}
