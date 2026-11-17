use anyhow::{Result, ensure};
use axum::http::{HeaderMap, header};

use crate::service::{Service, fault};

pub(super) fn check_origin(service: &Service, headers: &HeaderMap) -> Result<()> {
    ensure!(
        headers.get("sec-fetch-site").and_then(|h| h.to_str().ok()) != Some("cross-site"),
        fault(403, "FORBIDDEN", "Cross-site requests are not allowed")
    );
    if let Some(origin) = headers.get(header::ORIGIN) {
        let origin = origin
            .to_str()
            .map_err(|_| fault(403, "FORBIDDEN", "Invalid Origin"))?;
        let expected = url::Url::parse(&service.config.application.app_url)?
            .origin()
            .ascii_serialization();
        ensure!(
            origin == expected,
            fault(403, "FORBIDDEN", "Origin does not match APP_URL")
        );
    }
    Ok(())
}

pub(super) fn secure_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}
