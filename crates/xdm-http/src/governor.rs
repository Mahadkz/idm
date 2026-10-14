//! Hierarchical bandwidth control shared by every transfer in a process (N44-N47).
//!
//! Rate buckets pace by reservation: each chunk reserves a slot of `bytes / rate`
//! seconds on every applicable bucket (task, host, global) and waits for the latest
//! start. Reservations are first-come, so competing consumers alternate chunk by
//! chunk instead of one starving another. The hot path of an unlimited transfer is
//! a few relaxed atomic loads and never allocates.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Result, err};

/// Credit a slow consumer may bank, so scheduling jitter does not lose throughput.
const BURST: Duration = Duration::from_millis(50);
/// Longest sleep between checks for pause and live limit changes.
const SLICE: Duration = Duration::from_millis(25);
/// Per-host connection budget when no rule matches.
pub const DEFAULT_HOST_CONNECTIONS: usize = 16;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A pacing bucket. Rate 0 means unlimited.
#[derive(Debug)]
pub struct RateBucket {
    rate: AtomicU64,
    generation: AtomicU64,
    state: Mutex<Schedule>,
}

#[derive(Debug)]
struct Schedule {
    /// When the bucket as a whole is next free.
    next: Instant,
    /// Recently active consumers (tasks), each paced at an equal share.
    consumers: Vec<Consumer>,
}

#[derive(Debug)]
struct Consumer {
    id: u64,
    next: Instant,
    active_until: Instant,
}

/// How long a consumer keeps its fair share after its last reservation.
const ACTIVE_FOR: Duration = Duration::from_millis(300);

impl RateBucket {
    pub fn new(rate: Option<u64>) -> Self {
        Self {
            rate: AtomicU64::new(rate.unwrap_or(0)),
            generation: AtomicU64::new(0),
            state: Mutex::new(Schedule { next: Instant::now(), consumers: Vec::new() }),
        }
    }

    pub fn rate(&self) -> Option<u64> {
        Some(self.rate.load(Ordering::Relaxed)).filter(|rate| *rate > 0)
    }

    #[inline]
    pub(crate) fn unlimited(&self) -> bool {
        self.rate.load(Ordering::Relaxed) == 0
    }

    /// Changes the rate; waiters notice within one pacing slice and re-reserve.
    pub fn set(&self, rate: Option<u64>) {
        let mut state = locked(&self.state);
        self.rate.store(rate.unwrap_or(0), Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed);
        state.next = Instant::now();
        state.consumers.clear();
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Reserves `bytes` for `consumer` and returns when they may be released, or
    /// `None` if unlimited. Consumers active together each get an equal share of
    /// the rate, so one task with many connections cannot crowd out another.
    pub(crate) fn reserve(&self, consumer: u64, bytes: u64, now: Instant) -> Option<Instant> {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return None;
        }
        let mut state = locked(&self.state);
        let floor = now.checked_sub(BURST).unwrap_or(now);
        state.consumers.retain(|c| c.active_until > now || c.id == consumer);
        let index = match state.consumers.iter().position(|c| c.id == consumer) {
            Some(index) => index,
            None => {
                state.consumers.push(Consumer { id: consumer, next: floor, active_until: now });
                state.consumers.len() - 1
            }
        };
        let share = rate as f64 / state.consumers.len() as f64;
        let own = state.consumers[index].next.max(floor);
        let whole = state.next.max(floor);
        let start = own.max(whole);
        let slot = Duration::from_secs_f64(bytes as f64 / share);
        state.consumers[index].next = own + slot;
        state.consumers[index].active_until = own + slot + ACTIVE_FOR;
        state.next = whole + Duration::from_secs_f64(bytes as f64 / rate as f64);
        Some(start)
    }
}

/// Waits until `bytes` may pass every bucket. Pause (`stop`) interrupts the wait.
pub(crate) fn pace(
    buckets: [Option<&RateBucket>; 3],
    consumer: u64,
    bytes: usize,
    stop: &AtomicBool,
) -> Result<()> {
    loop {
        let now = Instant::now();
        let mut generations = 0u64;
        let mut deadline: Option<Instant> = None;
        for bucket in buckets.into_iter().flatten() {
            generations = generations.wrapping_add(bucket.generation());
            if let Some(start) = bucket.reserve(consumer, bytes as u64, now) {
                deadline = Some(deadline.map_or(start, |d| d.max(start)));
            }
        }
        let Some(deadline) = deadline else {
            return Ok(());
        };
        loop {
            if stop.load(Ordering::Relaxed) {
                return Err(err("paused"));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let changed = buckets
                .into_iter()
                .flatten()
                .fold(0u64, |sum, bucket| sum.wrapping_add(bucket.generation()))
                != generations;
            if changed {
                break;
            }
            thread::sleep((deadline - now).min(SLICE));
        }
    }
}

/// True when `pattern` covers `host`: exact, `*.suffix` (subdomains and the bare
/// domain) or `*`. Case-insensitive.
pub fn pattern_matches(pattern: &str, host: &str) -> bool {
    let (pattern, host) = (pattern.to_ascii_lowercase(), host.to_ascii_lowercase());
    if pattern == "*" || pattern == host {
        return true;
    }
    pattern
        .strip_prefix("*.")
        .is_some_and(|suffix| host == suffix || host.ends_with(&format!(".{suffix}")))
}

/// The rule for `host`: an exact pattern wins, then the longest wildcard, then `*`.
pub fn resolve<'a, T>(rules: &'a [(String, T)], host: &str) -> Option<&'a T> {
    rules
        .iter()
        .filter(|(pattern, _)| pattern_matches(pattern, host))
        .max_by_key(|(pattern, _)| {
            if pattern.eq_ignore_ascii_case(host) {
                usize::MAX
            } else if pattern == "*" {
                0
            } else {
                pattern.len()
            }
        })
        .map(|(_, value)| value)
}

pub fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

#[derive(Debug, Default)]
struct Rules {
    /// Host pattern -> KiB/s; 0 is an explicit "unlimited" that overrides wildcards.
    rates: Vec<(String, u64)>,
    budgets: Vec<(String, usize)>,
    buckets: HashMap<String, Arc<RateBucket>>,
    in_use: HashMap<String, usize>,
}

/// Point-in-time view for status commands.
#[derive(Debug, Clone, Default)]
pub struct GovernorSnapshot {
    pub global_kib: Option<u64>,
    pub host_rates: Vec<(String, u64)>,
    pub host_budgets: Vec<(String, usize)>,
    pub connections_in_use: Vec<(String, usize)>,
    pub quota_bytes: u64,
    pub quota_used: u64,
    pub quota_exceeded: bool,
}

/// Process-wide limits: global and per-host rate caps, per-host connection
/// budgets and the usage-quota counter.
#[derive(Debug)]
pub struct Governor {
    pub(crate) global: RateBucket,
    rules: Mutex<Rules>,
    freed: Condvar,
    used: AtomicU64,
    quota: AtomicU64,
    exceeded: AtomicBool,
}

impl Default for Governor {
    fn default() -> Self {
        Self {
            global: RateBucket::new(None),
            rules: Mutex::new(Rules::default()),
            freed: Condvar::new(),
            used: AtomicU64::new(0),
            quota: AtomicU64::new(0),
            exceeded: AtomicBool::new(false),
        }
    }
}

impl Governor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Global aggregate cap in KiB/s; `None` is unlimited.
    pub fn set_global_kib(&self, kib: Option<u64>) {
        self.global.set(kib.map(|kib| kib.saturating_mul(1024)));
    }

    pub fn global_kib(&self) -> Option<u64> {
        self.global.rate().map(|rate| rate / 1024)
    }

    fn host_rate(rules: &Rules, host: &str) -> Option<u64> {
        resolve(&rules.rates, host)
            .copied()
            .filter(|kib| *kib > 0)
            .map(|kib| kib.saturating_mul(1024))
    }

    /// Sets (`Some(kib)`; 0 is unlimited) or removes (`None`) a host pattern cap.
    /// Takes effect on running transfers within the pacing slice.
    pub fn set_host_limit(&self, pattern: &str, kib: Option<u64>) {
        let pattern = pattern.to_ascii_lowercase();
        let mut rules = locked(&self.rules);
        rules.rates.retain(|(existing, _)| *existing != pattern);
        if let Some(kib) = kib {
            rules.rates.push((pattern, kib));
        }
        for (host, bucket) in &rules.buckets {
            bucket.set(Self::host_rate(&rules, host));
        }
    }

    /// Sets (`Some(n >= 1)`) or removes (`None`) a host connection budget.
    pub fn set_host_connections(&self, pattern: &str, count: Option<usize>) {
        let pattern = pattern.to_ascii_lowercase();
        let mut rules = locked(&self.rules);
        rules.budgets.retain(|(existing, _)| *existing != pattern);
        if let Some(count) = count {
            rules.budgets.push((pattern, count.max(1)));
        }
        drop(rules);
        self.freed.notify_all();
    }

    pub(crate) fn host_bucket(&self, host: &str) -> Arc<RateBucket> {
        let mut rules = locked(&self.rules);
        if let Some(bucket) = rules.buckets.get(host) {
            return bucket.clone();
        }
        let bucket = Arc::new(RateBucket::new(Self::host_rate(&rules, host)));
        rules.buckets.insert(host.to_string(), bucket.clone());
        bucket
    }

    /// Waits for a connection lease on `host`; `stop` aborts the wait.
    pub fn acquire(self: &Arc<Self>, host: &str, stop: &AtomicBool) -> Result<Lease> {
        let host = host.to_ascii_lowercase();
        let mut rules = locked(&self.rules);
        loop {
            let limit = resolve(&rules.budgets, &host)
                .copied()
                .unwrap_or(DEFAULT_HOST_CONNECTIONS);
            let held = rules.in_use.entry(host.clone()).or_insert(0);
            if *held < limit {
                *held += 1;
                return Ok(Lease { governor: self.clone(), host });
            }
            if stop.load(Ordering::Relaxed) {
                return Err(err("paused"));
            }
            rules = self
                .freed
                .wait_timeout(rules, SLICE)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Counts payload bytes toward the quota and flags it when reached.
    #[inline]
    pub(crate) fn account(&self, bytes: u64) {
        let used = self.used.fetch_add(bytes, Ordering::Relaxed) + bytes;
        let quota = self.quota.load(Ordering::Relaxed);
        if quota > 0 && used >= quota {
            self.exceeded.store(true, Ordering::Relaxed);
        }
    }

    pub fn exceeded(&self) -> bool {
        self.exceeded.load(Ordering::Relaxed)
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    /// Quota in bytes; 0 disables it.
    pub fn set_quota(&self, bytes: u64) {
        self.quota.store(bytes, Ordering::Relaxed);
        self.exceeded
            .store(bytes > 0 && self.used() >= bytes, Ordering::Relaxed);
    }

    pub fn set_used(&self, bytes: u64) {
        self.used.store(bytes, Ordering::Relaxed);
        let quota = self.quota.load(Ordering::Relaxed);
        self.exceeded.store(quota > 0 && bytes >= quota, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> GovernorSnapshot {
        let rules = locked(&self.rules);
        let mut in_use: Vec<_> = rules
            .in_use
            .iter()
            .filter(|(_, count)| **count > 0)
            .map(|(host, count)| (host.clone(), *count))
            .collect();
        in_use.sort();
        GovernorSnapshot {
            global_kib: self.global_kib(),
            host_rates: rules.rates.clone(),
            host_budgets: rules.budgets.clone(),
            connections_in_use: in_use,
            quota_bytes: self.quota.load(Ordering::Relaxed),
            quota_used: self.used(),
            quota_exceeded: self.exceeded(),
        }
    }
}

/// One simultaneous connection to a host; released on drop.
#[derive(Debug)]
pub struct Lease {
    governor: Arc<Governor>,
    host: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut rules = locked(&self.governor.rules);
        if let Some(held) = rules.in_use.get_mut(&self.host) {
            *held = held.saturating_sub(1);
        }
        drop(rules);
        self.governor.freed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_queue_at_the_rate() {
        let bucket = RateBucket::new(Some(1000));
        let t0 = Instant::now();
        let first = bucket.reserve(0, 500, t0).unwrap();
        let second = bucket.reserve(0, 500, t0).unwrap();
        let third = bucket.reserve(0, 1000, t0).unwrap();
        assert_eq!(second - first, Duration::from_millis(500));
        assert_eq!(third - second, Duration::from_millis(500));
        // The schedule after `third` is one second long.
        let fourth = bucket.reserve(0, 1, t0).unwrap();
        assert_eq!(fourth - third, Duration::from_secs(1));
    }

    #[test]
    fn active_consumers_split_the_rate_equally() {
        let bucket = RateBucket::new(Some(1000));
        let now = Instant::now();
        // A greedy consumer that already queued ahead cannot keep more than half.
        bucket.reserve(1, 100, now);
        bucket.reserve(2, 100, now);
        let a1 = bucket.reserve(1, 100, now).unwrap();
        let a2 = bucket.reserve(1, 100, now).unwrap();
        let b1 = bucket.reserve(2, 100, now).unwrap();
        let b2 = bucket.reserve(2, 100, now).unwrap();
        // Neither consumer is served faster than half the rate once both are active,
        // and together they never exceed the whole rate.
        assert!(a2 - a1 >= Duration::from_millis(100));
        assert!(b2 - b1 >= Duration::from_millis(100));
        let last = a2.max(b2);
        assert!(last - now >= Duration::from_millis(450), "{:?}", last - now);
    }

    #[test]
    fn idle_credit_is_bounded_by_the_burst() {
        let bucket = RateBucket::new(Some(1000));
        let later = Instant::now() + Duration::from_secs(10);
        let start = bucket.reserve(0, 100, later).unwrap();
        assert_eq!(later - start, BURST);
    }

    #[test]
    fn unlimited_never_reserves_and_set_resets_schedule() {
        let bucket = RateBucket::new(None);
        assert!(bucket.reserve(0, 1 << 30, Instant::now()).is_none());
        bucket.set(Some(10));
        let now = Instant::now();
        bucket.reserve(0, 1000, now).unwrap();
        bucket.set(Some(10));
        let start = bucket.reserve(0, 1, now).unwrap();
        assert!(start <= Instant::now());
    }

    #[test]
    fn pace_returns_promptly_after_a_live_rate_change() {
        let bucket = Arc::new(RateBucket::new(Some(1)));
        let stop = Arc::new(AtomicBool::new(false));
        pace([Some(&bucket), None, None], 0, 1, &stop).unwrap();
        let waiting = (bucket.clone(), stop.clone());
        let handle = thread::spawn(move || {
            let begin = Instant::now();
            pace([Some(&waiting.0), None, None], 0, 100, &waiting.1).unwrap();
            begin.elapsed()
        });
        thread::sleep(Duration::from_millis(100));
        bucket.set(None);
        assert!(handle.join().unwrap() < Duration::from_millis(400));
    }

    #[test]
    fn host_patterns_prefer_exact_then_longest_wildcard() {
        let rules = vec![
            ("*".to_string(), 1),
            ("*.example.com".to_string(), 2),
            ("*.cdn.example.com".to_string(), 3),
            ("a.cdn.example.com".to_string(), 4),
        ];
        assert_eq!(resolve(&rules, "other.org"), Some(&1));
        assert_eq!(resolve(&rules, "www.example.com"), Some(&2));
        assert_eq!(resolve(&rules, "example.com"), Some(&2));
        assert_eq!(resolve(&rules, "x.cdn.example.com"), Some(&3));
        assert_eq!(resolve(&rules, "A.CDN.example.com"), Some(&4));
        assert!(!pattern_matches("*.example.com", "badexample.com"));
    }

    #[test]
    fn host_rule_changes_reach_existing_buckets() {
        let governor = Governor::new();
        let bucket = governor.host_bucket("a.test");
        assert!(bucket.rate().is_none());
        governor.set_host_limit("*", Some(8));
        assert_eq!(bucket.rate(), Some(8192));
        governor.set_host_limit("a.test", Some(0));
        assert!(bucket.rate().is_none());
        governor.set_host_limit("a.test", None);
        assert_eq!(bucket.rate(), Some(8192));
        assert_eq!(governor.host_bucket("b.test").rate(), Some(8192));
    }

    #[test]
    fn leases_never_exceed_the_budget_and_release_on_drop() {
        let governor = Governor::new();
        governor.set_host_connections("*", Some(2));
        let stop = AtomicBool::new(false);
        let a = governor.acquire("h.test", &stop).unwrap();
        let _b = governor.acquire("h.test", &stop).unwrap();
        // Another host has its own budget.
        let _c = governor.acquire("other.test", &stop).unwrap();
        let waiter = {
            let governor = governor.clone();
            thread::spawn(move || governor.acquire("h.test", &AtomicBool::new(false)).map(|_| ()))
        };
        thread::sleep(Duration::from_millis(100));
        assert_eq!(governor.snapshot().connections_in_use[0], ("h.test".into(), 2));
        drop(a);
        waiter.join().unwrap().unwrap();
        let stopping = AtomicBool::new(true);
        governor.set_host_connections("h.test", Some(1));
        assert!(governor.acquire("h.test", &stopping).is_err());
    }

    #[test]
    fn quota_flags_when_reached_and_clears_on_reset() {
        let governor = Governor::new();
        governor.account(500);
        governor.set_quota(1000);
        assert!(!governor.exceeded());
        governor.account(500);
        assert!(governor.exceeded());
        governor.set_used(0);
        assert!(!governor.exceeded());
        governor.account(2000);
        governor.set_quota(0);
        assert!(!governor.exceeded());
    }
}
