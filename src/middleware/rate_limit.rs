//! In-memory per-IP rate limiting for brute-force-sensitive auth routes.
//!
//! The limiter uses a fixed window per client key. It is intentionally simple
//! (no external dependency) and lives in process memory, which is enough to
//! stop credential stuffing from a single source address. Stale windows are
//! evicted lazily on every check so the map cannot grow without bound.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Fixed-window rate limiter keyed by client IP.
///
/// Each key gets its own window that starts on the first request and lasts
/// [`Self::window`]. Once [`Self::max`] requests are counted inside the
/// current window, further requests are rejected until the window rolls over.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, Window>>>,
    max: u32,
    window: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            // A limit of zero would reject everything, which is never intended.
            max: max.max(1),
            window,
        }
    }

    /// Record a request for `key`.
    ///
    /// Returns `Ok(())` when the request is allowed, or `Err(retry_after)`
    /// with the time remaining until the current window resets when the limit
    /// has been exceeded.
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        // A poisoned lock only means a previous caller panicked while holding
        // it; the map itself is still usable, so recover instead of panicking.
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        // Lazy eviction: drop windows whose period has fully elapsed.
        map.retain(|_, w| now.duration_since(w.started) < self.window);

        let window = map.entry(key.to_string()).or_insert(Window {
            started: now,
            count: 0,
        });

        if now.duration_since(window.started) >= self.window {
            window.started = now;
            window.count = 0;
        }

        if window.count >= self.max {
            return Err(self
                .window
                .saturating_sub(now.duration_since(window.started)));
        }

        window.count += 1;
        Ok(())
    }
}

/// Axum middleware that rejects requests over the per-IP limit with `429`.
///
/// Apply it to the auth routes only (via [`axum::Router::route_layer`]): a
/// global limit would break HTMX polling and normal browsing.
pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let key = crate::utils::client_ip(req.headers(), None).unwrap_or_else(|| "unknown".to_string());

    if let Err(retry_after) = limiter.check(&key) {
        let secs = retry_after.as_secs().max(1);
        let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
        if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
            response.headers_mut().insert(RETRY_AFTER, value);
        }
        return response;
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit_then_rejects() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        assert!(limiter.check("1.2.3.4").is_ok());
        assert!(limiter.check("1.2.3.4").is_ok());
        assert!(limiter.check("1.2.3.4").is_ok());
        assert!(limiter.check("1.2.3.4").is_err());
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("a").is_ok());
        assert!(limiter.check("b").is_ok());
        assert!(limiter.check("a").is_err());
    }

    #[test]
    fn window_resets_after_expiry() {
        let limiter = RateLimiter::new(1, Duration::from_millis(30));
        assert!(limiter.check("ip").is_ok());
        assert!(limiter.check("ip").is_err());
        std::thread::sleep(Duration::from_millis(40));
        assert!(limiter.check("ip").is_ok());
    }

    #[test]
    fn retry_after_is_bounded_by_window() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("ip").is_ok());
        let retry = limiter.check("ip").unwrap_err();
        assert!(retry > Duration::ZERO);
        assert!(retry <= Duration::from_secs(60));
    }

    #[test]
    fn stale_windows_are_evicted() {
        let limiter = RateLimiter::new(1, Duration::from_millis(10));
        assert!(limiter.check("old").is_ok());
        std::thread::sleep(Duration::from_millis(20));
        assert!(limiter.check("new").is_ok());

        let map = limiter.inner.lock().unwrap();
        assert!(!map.contains_key("old"), "expired window should be evicted");
        assert!(map.contains_key("new"));
    }
}
