//! Global and per-host limits, host connection budgets and the usage quota (N44-N47).

use super::*;
use xdm_http::{DEFAULT_HOST_CONNECTIONS, Governor};
use xdm_ipc::BandwidthReport;

/// Fixed-window usage quota. The window opens when the first HTTP task runs
/// after the previous window ended and lasts `window` seconds; payload bytes of
/// every HTTP task, in every queue, count. A transfer may overshoot the quota by
/// up to one network read per connection.
#[derive(Debug, Default)]
pub(crate) struct Quota {
    pub(crate) bytes: u64,
    pub(crate) window: i64,
    pub(crate) start: Option<i64>,
    persisted_used: u64,
}

fn load_rules<T: serde::de::DeserializeOwned>(db: &Connection, key: &str) -> Vec<(String, T)> {
    get_setting(db, key)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

/// Restores persisted limits and the quota ledger into a fresh governor.
pub(crate) fn load(db: &Connection, governor: &Governor) -> Quota {
    let number = |key: &str| {
        get_setting(db, key)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<i64>().ok())
    };
    governor.set_global_kib(number("bw_global_kib").map(|kib| kib as u64).filter(|kib| *kib > 0));
    for (pattern, kib) in load_rules::<u64>(db, "bw_host_limits") {
        governor.set_host_limit(&pattern, Some(kib));
    }
    for (pattern, count) in load_rules::<usize>(db, "bw_host_connections") {
        governor.set_host_connections(&pattern, Some(count));
    }
    let bytes = number("quota_mib").unwrap_or(0).max(0) as u64 * 1024 * 1024;
    let used = number("quota_used").unwrap_or(0).max(0) as u64;
    governor.set_quota(bytes);
    governor.set_used(used);
    Quota {
        bytes,
        window: number("quota_window_seconds").unwrap_or(0).max(0),
        start: number("quota_start"),
        persisted_used: used,
    }
}

/// Advances the quota window: opens it when HTTP work starts, closes it when it
/// ends, persists the ledger and wakes `quota_wait` tasks once usage is allowed.
pub(crate) fn quota_tick(inner: &mut Inner) -> Result<()> {
    let now = inner.clock.now();
    let governor = inner.governor.clone();
    if inner.quota.bytes == 0 {
        // Nothing is counted while the quota is off.
        inner.quota.start = None;
        governor.set_used(0);
    } else {
        if inner.quota.start.is_none()
            && (governor.used() > 0 || inner.active.values().any(|active| active.control.is_some()))
        {
            inner.quota.start = Some(now);
            set_setting(&inner.db, "quota_start", &now.to_string())?;
        }
        if inner.quota.start.is_some_and(|start| now >= start + inner.quota.window) {
            governor.set_used(0);
            inner.quota.start = None;
            inner.db.execute("DELETE FROM settings WHERE key='quota_start'", [])?;
        }
        let used = governor.used();
        if used != inner.quota.persisted_used {
            set_setting(&inner.db, "quota_used", &used.to_string())?;
            inner.quota.persisted_used = used;
        }
    }
    if !governor.exceeded() {
        inner.db.execute(
            "UPDATE tasks SET status='queued',updated_at=CURRENT_TIMESTAMP WHERE status='quota_wait'",
            [],
        )?;
    }
    Ok(())
}

/// Seconds until the quota window closes while usage is blocked or counted.
pub(crate) fn quota_due(inner: &Inner) -> Option<i64> {
    inner
        .quota
        .start
        .filter(|_| inner.quota.bytes > 0)
        .map(|start| start + inner.quota.window)
}

fn report(inner: &Inner) -> BandwidthReport {
    let snapshot = inner.governor.snapshot();
    let quota = &inner.quota;
    BandwidthReport {
        global_kib: snapshot.global_kib,
        host_limits: snapshot.host_rates,
        host_connections: snapshot
            .host_budgets
            .into_iter()
            .map(|(pattern, count)| (pattern, count as u64))
            .collect(),
        default_host_connections: DEFAULT_HOST_CONNECTIONS as u64,
        connections_in_use: snapshot
            .connections_in_use
            .into_iter()
            .map(|(host, count)| (host, count as u64))
            .collect(),
        quota_mib: quota.bytes / (1024 * 1024),
        quota_window_seconds: quota.window as u64,
        quota_used_bytes: snapshot.quota_used,
        quota_window_start: quota.start,
        quota_resets_at: quota_due(inner),
        quota_exceeded: snapshot.quota_exceeded,
        active_http_tasks: inner.active.values().filter(|a| a.control.is_some()).count() as u64,
    }
}

fn valid_pattern(pattern: &str) -> bool {
    !pattern.is_empty()
        && pattern.len() <= 253
        && pattern
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'*' | b'_'))
        && (!pattern.contains('*') || pattern == "*" || pattern.starts_with("*.") && !pattern[1..].contains('*'))
}

impl Service {
    pub(crate) fn bandwidth_command(&self, command: Command) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let governor = inner.governor.clone();
        match command {
            Command::SetGlobalLimit { kib } => {
                if kib.is_some_and(|kib| kib == 0 || kib > 1 << 22) {
                    return Ok(Reply::error("global limit must be 1 KiB/s to 4 GiB/s, or unlimited"));
                }
                governor.set_global_kib(kib);
                match kib {
                    Some(kib) => set_setting(&inner.db, "bw_global_kib", &kib.to_string())?,
                    None => {
                        inner.db.execute("DELETE FROM settings WHERE key='bw_global_kib'", [])?;
                    }
                }
            }
            Command::SetHostLimit { pattern, kib } => {
                if !valid_pattern(&pattern) {
                    return Ok(Reply::error("host pattern must be a host name, *.domain or *"));
                }
                if kib.is_some_and(|kib| kib > 1 << 22) {
                    return Ok(Reply::error("host limit must be at most 4 GiB/s"));
                }
                governor.set_host_limit(&pattern, kib);
                let rules = governor.snapshot().host_rates;
                set_setting(&inner.db, "bw_host_limits", &serde_json::to_string(&rules)?)?;
            }
            Command::SetHostConnections { pattern, count } => {
                if !valid_pattern(&pattern) {
                    return Ok(Reply::error("host pattern must be a host name, *.domain or *"));
                }
                if count.is_some_and(|count| !(1..=256).contains(&count)) {
                    return Ok(Reply::error("host connections must be between 1 and 256"));
                }
                governor.set_host_connections(&pattern, count.map(|count| count as usize));
                let rules = governor.snapshot().host_budgets;
                set_setting(&inner.db, "bw_host_connections", &serde_json::to_string(&rules)?)?;
            }
            Command::SetQuota { mib, window_seconds } => {
                if mib > 1 << 30 || (mib > 0 && !(1..=366 * 86_400).contains(&window_seconds)) {
                    return Ok(Reply::error("quota needs 1.. MiB and a window of 1 second to 366 days"));
                }
                let bytes = mib * 1024 * 1024;
                inner.quota.bytes = bytes;
                inner.quota.window = window_seconds as i64;
                // A new quota policy starts a fresh ledger.
                inner.quota.start = None;
                inner.db.execute("DELETE FROM settings WHERE key='quota_start'", [])?;
                governor.set_used(0);
                governor.set_quota(bytes);
                set_setting(&inner.db, "quota_mib", &mib.to_string())?;
                set_setting(&inner.db, "quota_window_seconds", &window_seconds.to_string())?;
            }
            Command::BandwidthStatus => {}
            _ => return Ok(Reply::error("not a bandwidth command")),
        }
        quota_tick(&mut inner)?;
        let mut reply = Reply::accepted(0);
        reply.bandwidth = Some(report(&inner));
        drop(inner);
        self.wake.notify_one();
        Ok(reply)
    }
}
