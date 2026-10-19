use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Jeton d'annulation partagé entre l'état (qui annule) et un worker (qui vérifie entre
/// chaque lot).
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
