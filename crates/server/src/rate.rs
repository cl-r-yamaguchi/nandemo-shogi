//! 接続ごとの送信頻度の制限（トークンバケット）と、不正なメッセージが続いたときの打ち切り。
//! 仕様は docs/specs/online.md の 6.4。時刻は、呼び出し側が渡す（時計に依存しない）。

use crate::Millis;

/// 一度に送れる数（バケットの大きさ）。
pub const BURST: u32 = 30;
/// 1 秒あたりに回復する数。
pub const PER_SECOND: u32 = 10;
/// 不正なメッセージ・頻度超過が、これだけ続いたら、接続を切る。
pub const MAX_STRIKES: u32 = 8;

/// トークンバケット: 1 メッセージにつき 1 つ使う。空なら拒否。
#[derive(Debug, Clone)]
pub struct RateLimiter {
    /// 千分の一トークン単位（小数を避ける）。
    millitokens: u64,
    last: Millis,
}

impl RateLimiter {
    pub fn new(now: Millis) -> Self {
        Self {
            millitokens: u64::from(BURST) * 1000,
            last: now,
        }
    }

    /// 1 メッセージを受けてよいか。
    pub fn allow(&mut self, now: Millis) -> bool {
        let elapsed = now.saturating_sub(self.last);
        self.last = self.last.max(now);
        let refill = elapsed.saturating_mul(u64::from(PER_SECOND));
        self.millitokens = (self.millitokens + refill).min(u64::from(BURST) * 1000);
        if self.millitokens >= 1000 {
            self.millitokens -= 1000;
            true
        } else {
            false
        }
    }
}

/// 不正なメッセージ・頻度超過の連続を数える。正しいメッセージで、数え直す。
#[derive(Debug, Clone, Default)]
pub struct Strikes {
    count: u32,
}

impl Strikes {
    /// 不正だった。上限に達したら `true`（接続を切る）。
    pub fn hit(&mut self) -> bool {
        self.count += 1;
        self.count >= MAX_STRIKES
    }

    /// 正しかった。
    pub fn ok(&mut self) {
        self.count = 0;
    }

    pub fn count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_is_allowed_then_refused_until_tokens_come_back() {
        let mut limiter = RateLimiter::new(0);
        for i in 0..BURST {
            assert!(limiter.allow(0), "{i} 番目");
        }
        assert!(!limiter.allow(0));
        // 100 ミリ秒で 1 トークン（毎秒 10）
        assert!(!limiter.allow(99));
        assert!(limiter.allow(100));
        assert!(!limiter.allow(100));
    }

    #[test]
    fn tokens_are_capped_at_the_burst_size() {
        let mut limiter = RateLimiter::new(0);
        // 長い時間がたっても、一度に使えるのは、バケットの大きさまで
        let now = 1_000_000;
        let allowed = (0..BURST * 2).filter(|_| limiter.allow(now)).count();
        assert_eq!(allowed as u32, BURST);
    }

    #[test]
    fn a_steady_rate_at_the_limit_is_always_allowed() {
        let mut limiter = RateLimiter::new(0);
        for i in 0..200u64 {
            assert!(limiter.allow(i * 100), "{i} 番目（毎秒 10 回）");
        }
    }

    #[test]
    fn going_back_in_time_does_not_refill() {
        let mut limiter = RateLimiter::new(10_000);
        for _ in 0..BURST {
            assert!(limiter.allow(10_000));
        }
        assert!(!limiter.allow(5_000));
    }

    #[test]
    fn strikes_cut_the_connection_only_when_they_continue() {
        let mut strikes = Strikes::default();
        for _ in 0..MAX_STRIKES - 1 {
            assert!(!strikes.hit());
        }
        strikes.ok();
        assert_eq!(strikes.count(), 0);
        for _ in 0..MAX_STRIKES - 1 {
            assert!(!strikes.hit());
        }
        assert!(strikes.hit());
    }
}
