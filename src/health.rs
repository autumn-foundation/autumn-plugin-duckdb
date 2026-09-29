//! The readiness check.
//!
//! # Contract
//!
//! - The check is down before the plugin starts and after the shutdown.
//! - The check runs `SELECT 1` on the root connection. A busy pool does not make it down.
//! - The output does not show the DuckDB message. The log has the error class.

use std::collections::HashMap;
use std::sync::Arc;

use autumn_web::actuator::{HealthCheckOutput, HealthIndicator};

use crate::plugin::Shared;

/// The future type of a health check.
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Runs `SELECT 1`.
pub(crate) struct DatabaseCheck {
    shared: Arc<Shared>,
}

impl DatabaseCheck {
    pub(crate) const fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl HealthIndicator for DatabaseCheck {
    fn check(&self) -> BoxFuture<'_, HealthCheckOutput> {
        Box::pin(async move {
            let Some(db) = self.shared.handle.get() else {
                return HealthCheckOutput::down().with_details(detail("state", "not started"));
            };
            let kind = if db.config().is_in_memory() {
                "in-memory"
            } else {
                "file"
            };
            match db.ping().await {
                Ok(()) => HealthCheckOutput::up().with_details(detail("database", kind)),
                Err(err) => {
                    tracing::warn!(class = err.class(), error = %err, "the DuckDB readiness check failed");
                    HealthCheckOutput::down().with_details(detail("database", kind))
                }
            }
        })
    }
}

fn detail(key: &str, value: &str) -> HashMap<String, serde_json::Value> {
    HashMap::from([(key.to_owned(), serde_json::Value::from(value))])
}

#[cfg(test)]
mod tests;
