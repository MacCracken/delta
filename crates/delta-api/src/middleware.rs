//! Rate limiting and request metrics middleware.

use axum::{extract::State, http::Request, response::Response};
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use crate::state::AppState;

/// Token-bucket rate limiter keyed by IP address.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<DashMap<String, (u32, Instant)>>,
    pub requests_per_window: u32,
    pub window_secs: u64,
}

impl RateLimiter {
    pub fn new(requests_per_window: u32, window_secs: u64) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            requests_per_window,
            window_secs,
        }
    }

    /// Check if a request from this IP is allowed. Returns remaining count, or `None` if blocked.
    pub fn check(&self, ip: &str) -> Option<u32> {
        let now = Instant::now();
        let mut entry = self.buckets.entry(ip.to_string()).or_insert((0, now));
        let (count, window_start) = entry.value_mut();

        if now.duration_since(*window_start).as_secs() >= self.window_secs {
            *count = 0;
            *window_start = now;
        }

        if *count >= self.requests_per_window {
            None
        } else {
            *count += 1;
            Some(self.requests_per_window - *count)
        }
    }

    /// Remove expired entries.
    pub fn cleanup(&self) {
        let now = Instant::now();
        self.buckets
            .retain(|_, (_, start)| now.duration_since(*start).as_secs() < self.window_secs * 2);
    }
}

/// Simple request metrics.
#[derive(Clone)]
pub struct Metrics {
    pub request_counts: Arc<DashMap<u16, u64>>,
    pub total_duration_us: Arc<AtomicU64>,
    pub total_requests: Arc<AtomicU64>,
    pub started_at: Instant,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            request_counts: Arc::new(DashMap::new()),
            total_duration_us: Arc::new(AtomicU64::new(0)),
            total_requests: Arc::new(AtomicU64::new(0)),
            started_at: Instant::now(),
        }
    }

    pub fn record(&self, status: u16, duration_us: u64) {
        *self.request_counts.entry(status).or_insert(0) += 1;
        self.total_duration_us
            .fetch_add(duration_us, std::sync::atomic::Ordering::Relaxed);
        self.total_requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Axum middleware that records request metrics (status code, latency) and
/// enforces the global rate limit. Applied to every request via the router.
pub async fn metrics_and_rate_limit(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    // --- rate limit ---
    if let Some(ref limiter) = state.rate_limiter {
        // Extract client IP from ConnectInfo or forwarded headers
        let ip = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
            .or_else(|| {
                req.extensions()
                    .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                    .map(|ci| ci.0.ip().to_string())
            })
            .unwrap_or_default();

        if !ip.is_empty() && limiter.check(&ip).is_none() {
            return Response::builder()
                .status(axum::http::StatusCode::TOO_MANY_REQUESTS)
                .header("retry-after", limiter.window_secs.to_string())
                .body(axum::body::Body::from("rate limit exceeded"))
                .unwrap();
        }
    }

    // --- metrics ---
    let start = Instant::now();
    let response = next.run(req).await;
    let duration_us = start.elapsed().as_micros() as u64;
    let status = response.status().as_u16();
    state.metrics.record(status, duration_us);

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let limiter = RateLimiter::new(5, 60);
        for i in 0..5 {
            assert!(
                limiter.check("127.0.0.1").is_some(),
                "request {} should be allowed",
                i
            );
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = RateLimiter::new(3, 60);
        assert!(limiter.check("1.2.3.4").is_some());
        assert!(limiter.check("1.2.3.4").is_some());
        assert!(limiter.check("1.2.3.4").is_some());
        assert!(limiter.check("1.2.3.4").is_none());
    }

    #[test]
    fn test_rate_limiter_separate_ips() {
        let limiter = RateLimiter::new(1, 60);
        assert!(limiter.check("1.1.1.1").is_some());
        assert!(limiter.check("2.2.2.2").is_some());
        assert!(limiter.check("1.1.1.1").is_none());
        assert!(limiter.check("2.2.2.2").is_none());
    }

    #[test]
    fn test_rate_limiter_returns_remaining() {
        let limiter = RateLimiter::new(5, 60);
        assert_eq!(limiter.check("10.0.0.1"), Some(4));
        assert_eq!(limiter.check("10.0.0.1"), Some(3));
        assert_eq!(limiter.check("10.0.0.1"), Some(2));
    }

    #[test]
    fn test_rate_limiter_cleanup() {
        let limiter = RateLimiter::new(5, 0); // 0-second window
        assert!(limiter.check("1.2.3.4").is_some());
        limiter.cleanup(); // Should remove expired
    }

    #[test]
    fn test_metrics_record() {
        let metrics = Metrics::new();
        metrics.record(200, 1000);
        metrics.record(200, 2000);
        metrics.record(404, 500);
        assert_eq!(*metrics.request_counts.get(&200).unwrap(), 2);
        assert_eq!(*metrics.request_counts.get(&404).unwrap(), 1);
        assert_eq!(
            metrics
                .total_requests
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
    }
}
