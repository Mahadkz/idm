//! Text and input helpers for the bandwidth and partial-file dialogs.

use xdm_ipc::{BandwidthReport, PartialEntry};

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Parses a whole-number field; blank means "none".
pub fn parse_optional_number(text: &str, what: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<u64>()
        .map(Some)
        .map_err(|_| format!("{what} must be a whole number"))
}

/// Quota fields to `(mib, window_seconds)`; blank or 0 MiB turns the quota off.
pub fn parse_quota(mib: &str, hours: &str) -> Result<(u64, u64), String> {
    let mib = parse_optional_number(mib, "Quota MiB")?.unwrap_or(0);
    if mib == 0 {
        return Ok((0, 3600));
    }
    let hours = parse_optional_number(hours, "Quota hours")?.unwrap_or(0);
    if !(1..=366 * 24).contains(&hours) {
        return Err("Quota window must be between 1 and 8784 hours".into());
    }
    Ok((mib, hours * 3600))
}

pub fn format_host_limits(rules: &[(String, u64)]) -> Vec<String> {
    rules
        .iter()
        .map(|(pattern, kib)| {
            if *kib == 0 {
                format!("{pattern}  ·  unlimited (overrides wildcard rules)")
            } else {
                format!("{pattern}  ·  {kib} KiB/s")
            }
        })
        .collect()
}

pub fn format_host_budgets(rules: &[(String, u64)]) -> Vec<String> {
    rules
        .iter()
        .map(|(pattern, count)| format!("{pattern}  ·  {count} connections"))
        .collect()
}

pub fn format_bandwidth_status(report: &BandwidthReport, now: i64) -> String {
    let mut lines = Vec::new();
    lines.push(match report.global_kib {
        Some(kib) => format!("Global limit: {kib} KiB/s"),
        None => "Global limit: unlimited".into(),
    });
    lines.push(format!("Active HTTP downloads: {}", report.active_http_tasks));
    lines.push(format!("Default connections per host: {}", report.default_host_connections));
    if report.connections_in_use.is_empty() {
        lines.push("Connections in use: none".into());
    } else {
        let used: Vec<String> = report
            .connections_in_use
            .iter()
            .map(|(host, count)| format!("{host} x{count}"))
            .collect();
        lines.push(format!("Connections in use: {}", used.join(", ")));
    }
    if report.quota_mib == 0 {
        lines.push("Quota: off".into());
    } else {
        let hours = report.quota_window_seconds as f64 / 3600.0;
        let mut line = format!(
            "Quota: {} MiB per {hours:.1} h, used {}",
            report.quota_mib,
            human_bytes(report.quota_used_bytes)
        );
        if let Some(reset) = report.quota_resets_at {
            let left = (reset - now).max(0);
            line.push_str(&format!(", resets in {}m {}s", left / 60, left % 60));
        }
        lines.push(line);
        if report.quota_exceeded {
            lines.push("Quota used up: downloads wait in quota_wait until the window resets.".into());
        }
    }
    lines.join("\n")
}

pub fn partial_label(entry: &PartialEntry) -> String {
    let mut parts = Vec::new();
    if entry.part.is_some() {
        parts.push(format!(".part {}", human_bytes(entry.part_size)));
    }
    if entry.meta.is_some() {
        parts.push(format!(".meta {}", human_bytes(entry.meta_size)));
    }
    format!("{}   ({})", entry.output, parts.join(" + "))
}

pub fn partial_paths(entry: &PartialEntry) -> Vec<String> {
    entry.part.iter().chain(entry.meta.iter()).cloned().collect()
}

pub fn partial_bytes(entry: &PartialEntry) -> u64 {
    entry.part_size + entry.meta_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_fields() {
        assert_eq!(parse_quota("", "24").unwrap(), (0, 3600));
        assert_eq!(parse_quota("0", "x").unwrap(), (0, 3600));
        assert_eq!(parse_quota("100", "2").unwrap(), (100, 7200));
        assert!(parse_quota("100", "0").is_err());
        assert!(parse_quota("abc", "1").is_err());
    }

    #[test]
    fn status_text_covers_quota_and_leases() {
        let report = BandwidthReport {
            global_kib: Some(512),
            quota_mib: 10,
            quota_window_seconds: 7200,
            quota_used_bytes: 10 * 1024 * 1024,
            quota_resets_at: Some(1100),
            quota_exceeded: true,
            connections_in_use: vec![("a.test".into(), 2)],
            default_host_connections: 16,
            ..Default::default()
        };
        let text = format_bandwidth_status(&report, 1000);
        assert!(text.contains("Global limit: 512 KiB/s"));
        assert!(text.contains("a.test x2"));
        assert!(text.contains("10 MiB per 2.0 h, used 10.0 MiB, resets in 1m 40s"));
        assert!(text.contains("quota_wait"));
        assert!(format_bandwidth_status(&BandwidthReport::default(), 0).contains("Quota: off"));
    }

    #[test]
    fn partial_entries_list_their_files() {
        let entry = PartialEntry {
            output: "/d/a.bin".into(),
            part: Some("/d/a.bin.part".into()),
            part_size: 2048,
            meta: Some("/d/a.bin.part.meta".into()),
            meta_size: 100,
        };
        assert_eq!(partial_paths(&entry), vec!["/d/a.bin.part", "/d/a.bin.part.meta"]);
        assert!(partial_label(&entry).contains(".part 2.0 KiB + .meta 100 B"));
        assert_eq!(partial_bytes(&entry), 2148);
        assert_eq!(format_host_limits(&[("*".into(), 0), ("h".into(), 5)]).len(), 2);
    }
}
