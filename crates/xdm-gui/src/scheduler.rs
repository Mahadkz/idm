use chrono::{Local, NaiveDateTime, TimeZone};
use xdm_ipc::{CompletionAction, CompletionPending, QueueEvent};

/// Parses "HH:MM" or a minute-count string into minutes since midnight (0..1439).
pub fn parse_clock(text: &str) -> Result<i64, &'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("time cannot be empty");
    }
    if let Some((hour_s, min_s)) = trimmed.split_once(':') {
        let hour: i64 = hour_s.parse().map_err(|_| "invalid hour in time")?;
        let min: i64 = min_s.parse().map_err(|_| "invalid minute in time")?;
        if !(0..24).contains(&hour) || !(0..60).contains(&min) {
            return Err("hour must be 00-23 and minute 00-59");
        }
        Ok(hour * 60 + min)
    } else if let Ok(minutes) = trimmed.parse::<i64>() {
        if (0..1440).contains(&minutes) {
            Ok(minutes)
        } else {
            Err("minutes since midnight must be 0 to 1439")
        }
    } else {
        Err("time must be in HH:MM format")
    }
}

/// Formats minutes since midnight (0..1439) as "HH:MM".
pub fn format_clock(minutes: i64) -> String {
    let clamped = minutes.rem_euclid(1440);
    format!("{:02}:{:02}", clamped / 60, clamped % 60)
}

/// Parses local "YYYY-MM-DD HH:MM" or raw Unix seconds into an epoch timestamp.
pub fn parse_once_datetime(text: &str) -> Result<i64, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("datetime cannot be empty".into());
    }
    if let Ok(unix) = trimmed.parse::<i64>() {
        if unix > 0 {
            return Ok(unix);
        }
    }
    for fmt in ["%Y-%m-%d %H:%M", "%Y-%m-%d %H:%M:%S", "%Y/%m/%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, fmt) {
            if let Some(local) = Local.from_local_datetime(&naive).single() {
                return Ok(local.timestamp());
            }
        }
    }
    Err("expected local date/time YYYY-MM-DD HH:MM or Unix timestamp".into())
}

/// Formats an epoch timestamp as "YYYY-MM-DD HH:MM".
pub fn format_once_datetime(unix_secs: i64) -> String {
    if unix_secs <= 0 {
        return String::new();
    }
    Local
        .timestamp_opt(unix_secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| unix_secs.to_string())
}

/// Parses retry count where -1 or "infinite" indicates unlimited retries.
pub fn parse_retries(text: &str, infinite: bool) -> Result<i64, &'static str> {
    if infinite {
        return Ok(-1);
    }
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("infinite") || trimmed == "-1" {
        return Ok(-1);
    }
    let count: i64 = trimmed.parse().map_err(|_| "retries must be an integer or infinite")?;
    if (-1..=1000).contains(&count) {
        Ok(count)
    } else {
        Err("retries must be between 0 and 1000 (or -1 for infinite)")
    }
}

/// Assembles a bitmask where Monday=bit 0 ... Sunday=bit 6.
pub fn days_mask(mon: bool, tue: bool, wed: bool, thu: bool, fri: bool, sat: bool, sun: bool) -> u8 {
    let mut mask = 0u8;
    if mon { mask |= 1 << 0; }
    if tue { mask |= 1 << 1; }
    if wed { mask |= 1 << 2; }
    if thu { mask |= 1 << 3; }
    if fri { mask |= 1 << 4; }
    if sat { mask |= 1 << 5; }
    if sun { mask |= 1 << 6; }
    mask
}

/// Unpacks a Monday=bit 0 ... Sunday=bit 6 bitmask into individual flags.
pub fn unpack_days_mask(mask: u8) -> (bool, bool, bool, bool, bool, bool, bool) {
    (
        (mask & (1 << 0)) != 0,
        (mask & (1 << 1)) != 0,
        (mask & (1 << 2)) != 0,
        (mask & (1 << 3)) != 0,
        (mask & (1 << 4)) != 0,
        (mask & (1 << 5)) != 0,
        (mask & (1 << 6)) != 0,
    )
}

/// Formats a QueueEvent into a human-readable display string.
pub fn format_event(event: &QueueEvent) -> String {
    let time_str = Local
        .timestamp_opt(event.at, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| event.at.to_string());
    format!(
        "{} · exec #{} · outcome: {} · action: {} ({})",
        time_str, event.execution, event.outcome, event.action, event.action_status
    )
}

/// Parses countdown seconds ensuring it is within the 10..=3600 range.
pub fn parse_countdown(text: &str) -> Result<u64, &'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(60);
    }
    match trimmed.parse::<u64>() {
        Ok(secs) if (10..=3600).contains(&secs) => Ok(secs),
        Ok(_) => Err("countdown must be between 10 and 3600 seconds"),
        Err(_) => Err("countdown must be an integer"),
    }
}

/// Splits a multiline string into literal program arguments, one per non-empty line.
pub fn parse_program_args(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect()
}

/// Constructs a CompletionAction from UI fields, performing basic validation.
pub fn build_completion_action(
    action_name: &str,
    program_path: &str,
    program_args_text: &str,
) -> Result<CompletionAction, String> {
    match action_name.to_ascii_lowercase().as_str() {
        "none" => Ok(CompletionAction::None),
        "notify" => Ok(CompletionAction::Notify),
        "sleep" => Ok(CompletionAction::Sleep),
        "hibernate" => Ok(CompletionAction::Hibernate),
        "shutdown" => Ok(CompletionAction::Shutdown),
        "restart" => Ok(CompletionAction::Restart),
        "exit" => Ok(CompletionAction::Exit),
        "program" => {
            let path = program_path.trim();
            if path.is_empty() {
                return Err("program executable path cannot be empty".into());
            }
            if path.len() > 4096 || path.contains('\0') {
                return Err("program executable path is invalid".into());
            }
            let args = parse_program_args(program_args_text);
            if args.len() > 64 {
                return Err("maximum 64 arguments supported".into());
            }
            for arg in &args {
                if arg.len() > 4096 || arg.contains('\0') {
                    return Err("argument exceeds maximum allowed length".into());
                }
            }
            Ok(CompletionAction::Program {
                program: path.to_string(),
                args,
            })
        }
        other => Err(format!("unknown completion action '{other}'")),
    }
}

/// Validates that destructive actions have received explicit confirmation.
pub fn validate_destructive_confirmation(
    action: &CompletionAction,
    confirmed: bool,
) -> Result<(), String> {
    if action.is_destructive() && !confirmed {
        let action_name = match action {
            CompletionAction::Sleep => "sleep",
            CompletionAction::Hibernate => "hibernate",
            CompletionAction::Shutdown => "shutdown",
            CompletionAction::Restart => "restart",
            _ => "modify",
        };
        return Err(format!(
            "you must confirm: I understand this will {action_name} the computer"
        ));
    }
    Ok(())
}

/// Formats the countdown banner for active pending completion actions.
pub fn format_countdown_banner(
    pending: &[CompletionPending],
) -> Option<(String, i64, u64)> {
    if pending.is_empty() {
        return None;
    }
    let first = &pending[0];
    let label = if pending.len() == 1 {
        format!(
            "System will {} in {}s (Queue #{})",
            first.action, first.seconds_remaining, first.queue_id
        )
    } else {
        format!(
            "System will {} in {}s (Queue #{}) [+{} more]",
            first.action,
            first.seconds_remaining,
            first.queue_id,
            pending.len() - 1
        )
    };
    Some((label, first.queue_id, first.seconds_remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_clock() {
        assert_eq!(parse_clock("00:00").unwrap(), 0);
        assert_eq!(parse_clock("09:30").unwrap(), 570);
        assert_eq!(parse_clock("23:59").unwrap(), 1439);
        assert_eq!(parse_clock("120").unwrap(), 120);
        assert!(parse_clock("24:00").is_err());
        assert!(parse_clock("12:60").is_err());
        assert!(parse_clock("invalid").is_err());
    }

    #[test]
    fn test_format_clock() {
        assert_eq!(format_clock(0), "00:00");
        assert_eq!(format_clock(570), "09:30");
        assert_eq!(format_clock(1439), "23:59");
    }

    #[test]
    fn test_parse_once_datetime() {
        assert_eq!(parse_once_datetime("1700000000").unwrap(), 1700000000);
        assert!(parse_once_datetime("2026-10-13 10:00").is_ok());
        assert!(parse_once_datetime("invalid-date").is_err());
    }

    #[test]
    fn test_parse_retries() {
        assert_eq!(parse_retries("3", false).unwrap(), 3);
        assert_eq!(parse_retries("0", false).unwrap(), 0);
        assert_eq!(parse_retries("anything", true).unwrap(), -1);
        assert_eq!(parse_retries("infinite", false).unwrap(), -1);
        assert_eq!(parse_retries("-1", false).unwrap(), -1);
        assert!(parse_retries("1001", false).is_err());
        assert!(parse_retries("-2", false).is_err());
    }

    #[test]
    fn test_days_mask_roundtrip() {
        let mask_all = days_mask(true, true, true, true, true, true, true);
        assert_eq!(mask_all, 0x7f);
        assert_eq!(unpack_days_mask(mask_all), (true, true, true, true, true, true, true));

        let mask_weekdays = days_mask(true, true, true, true, true, false, false);
        assert_eq!(mask_weekdays, 0x1f);
        assert_eq!(unpack_days_mask(mask_weekdays), (true, true, true, true, true, false, false));

        let mask_custom = days_mask(true, false, true, false, false, true, false);
        assert_eq!(unpack_days_mask(mask_custom), (true, false, true, false, false, true, false));
    }

    #[test]
    fn test_format_event() {
        let event = QueueEvent {
            id: 1,
            queue_id: 2,
            execution: 3,
            outcome: "completed".into(),
            action: "shutdown".into(),
            action_status: "recorded".into(),
            at: 1700000000,
        };
        let formatted = format_event(&event);
        assert!(formatted.contains("exec #3"));
        assert!(formatted.contains("outcome: completed"));
        assert!(formatted.contains("action: shutdown (recorded)"));
    }

    #[test]
    fn test_parse_countdown() {
        assert_eq!(parse_countdown("").unwrap(), 60);
        assert_eq!(parse_countdown("   ").unwrap(), 60);
        assert_eq!(parse_countdown("10").unwrap(), 10);
        assert_eq!(parse_countdown("60").unwrap(), 60);
        assert_eq!(parse_countdown("3600").unwrap(), 3600);
        assert!(parse_countdown("9").is_err());
        assert!(parse_countdown("3601").is_err());
        assert!(parse_countdown("0").is_err());
        assert!(parse_countdown("abc").is_err());
    }

    #[test]
    fn test_parse_program_args() {
        assert_eq!(parse_program_args(""), Vec::<String>::new());
        assert_eq!(parse_program_args("   \n\n"), Vec::<String>::new());
        let multiline = "hello world\n--flag\n  spaced  \r\narg with ; special";
        assert_eq!(
            parse_program_args(multiline),
            vec!["hello world", "--flag", "  spaced  ", "arg with ; special"]
        );
    }

    #[test]
    fn test_build_completion_action() {
        assert_eq!(build_completion_action("none", "", "").unwrap(), CompletionAction::None);
        assert_eq!(build_completion_action("notify", "", "").unwrap(), CompletionAction::Notify);
        assert_eq!(build_completion_action("sleep", "", "").unwrap(), CompletionAction::Sleep);
        assert_eq!(build_completion_action("hibernate", "", "").unwrap(), CompletionAction::Hibernate);
        assert_eq!(build_completion_action("shutdown", "", "").unwrap(), CompletionAction::Shutdown);
        assert_eq!(build_completion_action("restart", "", "").unwrap(), CompletionAction::Restart);
        assert_eq!(build_completion_action("exit", "", "").unwrap(), CompletionAction::Exit);

        let prog = build_completion_action("program", "/bin/echo", "arg1\narg2").unwrap();
        assert_eq!(
            prog,
            CompletionAction::Program {
                program: "/bin/echo".into(),
                args: vec!["arg1".into(), "arg2".into()],
            }
        );

        assert!(build_completion_action("program", "", "arg1").is_err());
        assert!(build_completion_action("unknown_action", "", "").is_err());
    }

    #[test]
    fn test_validate_destructive_confirmation() {
        let sleep = CompletionAction::Sleep;
        assert!(validate_destructive_confirmation(&sleep, false).is_err());
        assert!(validate_destructive_confirmation(&sleep, true).is_ok());

        let shutdown = CompletionAction::Shutdown;
        assert!(validate_destructive_confirmation(&shutdown, false).is_err());
        assert!(validate_destructive_confirmation(&shutdown, true).is_ok());

        let notify = CompletionAction::Notify;
        assert!(validate_destructive_confirmation(&notify, false).is_ok());
        assert!(validate_destructive_confirmation(&notify, true).is_ok());

        let prog = CompletionAction::Program { program: "ls".into(), args: vec![] };
        assert!(validate_destructive_confirmation(&prog, false).is_ok());
    }

    #[test]
    fn test_format_countdown_banner() {
        assert!(format_countdown_banner(&[]).is_none());
        let single = vec![CompletionPending {
            queue_id: 2,
            action: "shutdown".into(),
            deadline_unix: 1700000000,
            seconds_remaining: 45,
        }];
        let (label, q_id, secs) = format_countdown_banner(&single).unwrap();
        assert_eq!(q_id, 2);
        assert_eq!(secs, 45);
        assert!(label.contains("shutdown"));
        assert!(label.contains("45s"));
        assert!(label.contains("Queue #2"));

        let multiple = vec![
            CompletionPending {
                queue_id: 1,
                action: "sleep".into(),
                deadline_unix: 1700000000,
                seconds_remaining: 20,
            },
            CompletionPending {
                queue_id: 2,
                action: "shutdown".into(),
                deadline_unix: 1700000010,
                seconds_remaining: 30,
            },
        ];
        let (label2, q_id2, secs2) = format_countdown_banner(&multiple).unwrap();
        assert_eq!(q_id2, 1);
        assert_eq!(secs2, 20);
        assert!(label2.contains("+1 more"));
    }
}
