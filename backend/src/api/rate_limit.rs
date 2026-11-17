use std::time::{Duration, Instant};

use anyhow::{Result, ensure};

use crate::service::fault;

use super::ApiState;

pub(crate) struct RateWindow {
    pub(super) count: u32,
    pub(super) expires_at: Instant,
}

pub(super) fn rate(state: &ApiState, key: String, limit: u32, window: Duration) -> Result<()> {
    let mut rates = state
        .rates
        .lock()
        .map_err(|_| fault(500, "INTERNAL_ERROR", "Rate limiter unavailable"))?;
    let maximum = state.service.config.rate_limits.maximum_clients;
    if rates.len() >= maximum {
        rates.retain(|_, entry| entry.expires_at > Instant::now());
    }
    ensure!(
        rates.len() < maximum || rates.contains_key(&key),
        fault(429, "RATE_LIMITED", "Too many active clients")
    );
    let entry = rates.entry(key).or_insert_with(|| RateWindow {
        count: 0,
        expires_at: Instant::now() + window,
    });
    if Instant::now() > entry.expires_at {
        entry.count = 0;
        entry.expires_at = Instant::now() + window;
    }
    entry.count += 1;
    ensure!(
        entry.count <= limit,
        fault(429, "RATE_LIMITED", "Too many requests. Try again later.")
    );
    Ok(())
}
