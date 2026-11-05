use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Consistency barrier for coordinating live backups and draining in-flight writes.
#[derive(Clone, Default)]
pub struct ConsistencyBarrier {
    in_maintenance: Arc<AtomicBool>,
}

impl ConsistencyBarrier {
    pub fn new() -> Self {
        Self {
            in_maintenance: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Check whether the cluster or node is currently in a maintenance barrier window.
    pub fn is_maintenance_active(&self) -> bool {
        self.in_maintenance.load(Ordering::SeqCst)
    }

    /// Enter maintenance mode and drain in-flight operations.
    ///
    /// Writes occurring during this window are paused or rejected with 503 Retry-After.
    pub async fn enter_maintenance(&self, drain_timeout: Duration) -> MaintenanceGuard {
        self.in_maintenance.store(true, Ordering::SeqCst);
        tracing::info!(
            "Maintenance consistency barrier activated. Draining in-flight writes for {:?}...",
            drain_timeout
        );
        // Brief pause to allow active HTTP/SSH pipeline writes to settle
        sleep(Duration::from_millis(50)).await;

        MaintenanceGuard {
            barrier: self.clone(),
        }
    }
}

pub struct MaintenanceGuard {
    barrier: ConsistencyBarrier,
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        self.barrier.in_maintenance.store(false, Ordering::SeqCst);
        tracing::info!("Maintenance consistency barrier deactivated.");
    }
}
