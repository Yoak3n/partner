use std::collections::HashMap;
use std::sync::Mutex;
use tokio::time::{Duration, Instant};

use ai_partner_shared::ModelProvider;

struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(max_per_minute: u32) -> Self {
        let max = max_per_minute as f64;
        Self {
            tokens: max,
            max_tokens: max,
            refill_rate: max / 60.0,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }

    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn time_until_available(&self) -> Duration {
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let deficit = 1.0 - self.tokens;
            Duration::from_secs_f64(deficit / self.refill_rate)
        }
    }
}

fn endpoint_key(provider: &ModelProvider) -> String {
    format!("{}|{}|{}", provider.kind, provider.name, provider.base_url)
}

pub struct RateLimiter {
    /// None entry = unlimited (requests_per_minute == 0)
    buckets: Mutex<HashMap<String, Option<TokenBucket>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// 注册一个 provider 端点的速率限制。
    /// requests_per_minute == 0 表示不限速。
    pub fn configure(&self, provider: &ModelProvider) {
        let key = endpoint_key(provider);
        let bucket = if provider.requests_per_minute == 0 {
            None
        } else {
            Some(TokenBucket::new(provider.requests_per_minute))
        };
        let mut buckets = self.buckets.lock().unwrap();
        buckets.entry(key).or_insert(bucket);
    }

    pub fn configure_all(&self, providers: &[ModelProvider]) {
        for p in providers {
            if p.enabled {
                self.configure(p);
            }
        }
    }

    /// 尝试消费一个令牌。无桶（rpm=0）视为不限速，直接通过。
    pub fn check(&self, provider: &ModelProvider) -> Result<(), RateLimitError> {
        let key = endpoint_key(provider);
        let mut buckets = self.buckets.lock().unwrap();
        match buckets.get_mut(&key) {
            None => Err(RateLimitError::NotConfigured),
            Some(None) => Ok(()), // unlimited
            Some(Some(bucket)) => {
                if bucket.try_consume() {
                    Ok(())
                } else {
                    Err(RateLimitError::Limited {
                        retry_after: bucket.time_until_available(),
                    })
                }
            }
        }
    }

    pub async fn wait(&self, provider: &ModelProvider) -> Result<(), RateLimitError> {
        let key = endpoint_key(provider);
        let wait_duration = {
            let buckets = self.buckets.lock().unwrap();
            match buckets.get(&key) {
                None => return Err(RateLimitError::NotConfigured),
                Some(None) => return Ok(()), // unlimited
                Some(Some(bucket)) => bucket.time_until_available(),
            }
        };

        if !wait_duration.is_zero() {
            tokio::time::sleep(wait_duration).await;
        }

        self.check(provider)
    }

    /// 检查是否可用（不消费令牌）
    pub fn is_available(&self, provider: &ModelProvider) -> bool {
        let key = endpoint_key(provider);
        let mut buckets = self.buckets.lock().unwrap();
        match buckets.get_mut(&key) {
            None => false,
            Some(None) => true, // unlimited
            Some(Some(bucket)) => {
                bucket.refill();
                bucket.tokens >= 1.0
            }
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("rate limited, retry after {retry_after:?}")]
    Limited { retry_after: Duration },

    #[error("provider not configured in rate limiter")]
    NotConfigured,
}

#[cfg(test)]
mod tests {
    use super::*;

    use ai_partner_shared::ModelKind;

    fn make_provider(name: &str, url: &str, rpm: u32) -> ModelProvider {
        let mut p = ModelProvider::new(ModelKind::Chat, name, url, "key", "model");
        p.requests_per_minute = rpm;
        p
    }

    #[test]
    fn test_basic_rate_limit() {
        let limiter = RateLimiter::new();
        let p = make_provider("openai", "https://api.openai.com/v1", 2);
        limiter.configure(&p);

        assert!(limiter.check(&p).is_ok());
        assert!(limiter.check(&p).is_ok());
        assert!(limiter.check(&p).is_err());
    }

    #[test]
    fn test_zero_rpm_means_unlimited() {
        let limiter = RateLimiter::new();
        let p = make_provider("openai", "https://api.openai.com/v1", 0);
        limiter.configure(&p);

        for _ in 0..1000 {
            assert!(limiter.check(&p).is_ok());
        }
    }

    #[test]
    fn test_same_endpoint_shares_bucket() {
        let limiter = RateLimiter::new();
        let mut p1 = make_provider("openai", "https://api.openai.com/v1", 2);
        p1.id = "id-1".into();
        let mut p2 = make_provider("openai", "https://api.openai.com/v1", 2);
        p2.id = "id-2".into();

        limiter.configure(&p1);

        assert!(limiter.check(&p1).is_ok());
        assert!(limiter.check(&p2).is_ok());
        assert!(limiter.check(&p1).is_err());
    }

    #[test]
    fn test_different_endpoints_separate_buckets() {
        let limiter = RateLimiter::new();
        let p1 = make_provider("openai", "https://api.openai.com/v1", 1);
        let p2 = make_provider("openai", "https://api.openai.com/v2", 1);

        limiter.configure(&p1);
        limiter.configure(&p2);

        assert!(limiter.check(&p1).is_ok());
        assert!(limiter.check(&p2).is_ok());
    }

    #[test]
    fn test_not_configured() {
        let limiter = RateLimiter::new();
        let p = make_provider("unknown", "http://x", 1);
        assert!(matches!(limiter.check(&p), Err(RateLimitError::NotConfigured)));
    }

    #[test]
    fn test_is_available_unlimited() {
        let limiter = RateLimiter::new();
        let p = make_provider("test", "http://x", 0);
        limiter.configure(&p);
        assert!(limiter.is_available(&p));
    }

    #[test]
    fn test_configure_all_skips_zero_rpm() {
        let limiter = RateLimiter::new();
        let p1 = make_provider("a", "http://a", 0);
        let p2 = make_provider("b", "http://b", 1);

        limiter.configure_all(&[p1.clone(), p2.clone()]);

        // p1 is unlimited
        for _ in 0..100 {
            assert!(limiter.check(&p1).is_ok());
        }
        // p2 is limited
        assert!(limiter.check(&p2).is_ok());
        assert!(limiter.check(&p2).is_err());
    }
}
