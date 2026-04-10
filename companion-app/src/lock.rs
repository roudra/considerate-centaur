// Per-learner read-write lock manager.
//
// Allows concurrent reads but exclusive writes per learner. Prevents
// file corruption when multiple requests touch the same learner's data
// simultaneously (e.g. session completion writing progress while the
// dashboard reads it).
//
// Usage:
//   let guard = lock_manager.read(learner_id).await;
//   // ... read files ...
//   drop(guard);
//
//   let guard = lock_manager.write(learner_id).await;
//   // ... write files ...
//   drop(guard);

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, OwnedRwLockReadGuard, OwnedRwLockWriteGuard};
use uuid::Uuid;

/// Manages per-learner read-write locks.
///
/// Each learner gets their own `RwLock` — multiple readers can proceed
/// concurrently, but a writer gets exclusive access. Locks for different
/// learners are fully independent.
#[derive(Clone, Default)]
pub struct LockManager {
    locks: Arc<Mutex<HashMap<Uuid, Arc<RwLock<()>>>>>,
}

impl LockManager {
    pub fn new() -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get or create the lock for a specific learner.
    async fn get_lock(&self, learner_id: Uuid) -> Arc<RwLock<()>> {
        let mut map = self.locks.lock().await;
        map.entry(learner_id)
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    /// Acquire a read lock for a learner. Multiple readers can hold this
    /// concurrently. Blocks only if a writer holds the lock.
    pub async fn read(&self, learner_id: Uuid) -> OwnedRwLockReadGuard<()> {
        let lock = self.get_lock(learner_id).await;
        lock.read_owned().await
    }

    /// Acquire a write lock for a learner. Exclusive — blocks all other
    /// readers and writers for this learner until dropped.
    pub async fn write(&self, learner_id: Uuid) -> OwnedRwLockWriteGuard<()> {
        let lock = self.get_lock(learner_id).await;
        lock.write_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_concurrent_reads_allowed() {
        let manager = LockManager::new();
        let id = Uuid::new_v4();

        let guard1 = manager.read(id).await;
        let guard2 = manager.read(id).await;

        // Both read guards held simultaneously — no deadlock.
        drop(guard1);
        drop(guard2);
    }

    #[tokio::test]
    async fn test_write_is_exclusive() {
        let manager = LockManager::new();
        let id = Uuid::new_v4();
        let counter = Arc::new(AtomicU32::new(0));

        let m1 = manager.clone();
        let c1 = counter.clone();
        let h1 = tokio::spawn(async move {
            let _guard = m1.write(id).await;
            let val = c1.fetch_add(1, Ordering::SeqCst);
            // While we hold the write lock, no other writer should have incremented.
            tokio::task::yield_now().await;
            assert_eq!(c1.load(Ordering::SeqCst), val + 1);
        });

        let m2 = manager.clone();
        let c2 = counter.clone();
        let h2 = tokio::spawn(async move {
            let _guard = m2.write(id).await;
            let val = c2.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            assert_eq!(c2.load(Ordering::SeqCst), val + 1);
        });

        h1.await.unwrap();
        h2.await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_different_learners_independent() {
        let manager = LockManager::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        // Write lock on learner 1 does not block read on learner 2.
        let _write_guard = manager.write(id1).await;
        let _read_guard = manager.read(id2).await;
    }
}
