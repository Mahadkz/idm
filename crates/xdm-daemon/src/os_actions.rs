//! OS action adapters for queue completion actions: desktop notifications,
//! power transitions, program launch and daemon exit. The adapter is
//! injectable; the real implementation runs platform tools directly (no
//! shell), while the fake records everything to a JSON file so tests can
//! verify behavior without ever touching the OS.

use crate::Result;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use xdm_ipc::CompletionAction;

/// Executes one recorded completion action. `armed` is true only when a
/// destructive action's cancelable countdown has elapsed; destructive actions
/// must never execute with `armed` false (they were recorded without the
/// explicit opt-in). The returned string is stored as the queue event's
/// action status.
pub trait CompletionAdapter: Send + Sync {
    fn run(&self, queue_id: i64, action: &str, armed: bool) -> String;
}

/// Inert recording adapter (tests only): the historic "recorded only" behavior.
#[cfg(test)]
pub struct RecordOnly;

#[cfg(test)]
impl CompletionAdapter for RecordOnly {
    fn run(&self, _queue_id: i64, _action: &str, _armed: bool) -> String {
        "recorded".into()
    }
}

/// Real OS adapter. Every command is spawned directly with explicit
/// arguments — never through a shell.
pub struct RealOs;

impl CompletionAdapter for RealOs {
    fn run(&self, queue_id: i64, action: &str, armed: bool) -> String {
        let Some(parsed) = CompletionAction::decode(action) else {
            return "failed: unknown action".into();
        };
        // Destructive power states require the opted-in countdown.
        if parsed.is_destructive() && !armed {
            return "recorded".into();
        }
        match parsed {
            // Launched without waiting so a long-running program never holds the daemon lock;
            // a reaper thread collects its exit status.
            CompletionAction::Program { program, args } => {
                match Command::new(&program).args(&args).spawn() {
                    Ok(mut child) => {
                        let _ = thread::Builder::new()
                            .name("xdm-program".into())
                            .spawn(move || child.wait());
                        "started".into()
                    }
                    Err(error) => format!("failed: {program}: {error}"),
                }
            }
            CompletionAction::Exit => {
                // Leave a short grace period so the event insert and the
                // pending reply frame are written before the process exits.
                match thread::Builder::new()
                    .name("xdm-exit".into())
                    .spawn(|| {
                        thread::sleep(Duration::from_millis(300));
                        std::process::exit(0);
                    })
                {
                    Ok(_) => "executed".to_string(),
                    Err(error) => format!("failed: {error}"),
                }
            }
            CompletionAction::Notify => notify(queue_id),
            CompletionAction::None => "none".into(),
            power => power_action(&power),
        }
    }
}

fn run_process_status(program: &str, args: &[&str]) -> String {
    run_process(program, args)
        .map(|()| "executed".to_string())
        .unwrap_or_else(|error| error.to_string())
}

fn run_process(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("{program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}").into())
    }
}

/// Toast through the Windows Runtime; falls back to `msg` when PowerShell
/// is unavailable or the toast fails.
#[cfg(windows)]
fn notify(queue_id: i64) -> String {
    let body = format!("Download queue {queue_id} completed").replace('\'', "''");
    let script = format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
         $t=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
         $t.GetElementsByTagName('text').Item(0).AppendChild($t.CreateTextNode('XDM')) | Out-Null; \
         $t.GetElementsByTagName('text').Item(1).AppendChild($t.CreateTextNode('{body}')) | Out-Null; \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('XDM Download Manager').Show([Windows.UI.Notifications.ToastNotification]::new($t))"
    );
    match Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
    {
        Ok(status) if status.success() => "executed".into(),
        _ => run_process_status("msg", &["*", &format!("Download queue {queue_id} completed")]),
    }
}

#[cfg(windows)]
fn power_action(action: &CompletionAction) -> String {
    let (program, args): (&str, Vec<&str>) = match action {
        CompletionAction::Sleep => ("rundll32", vec!["powrprof.dll,SetSuspendState", "0,1,0"]),
        CompletionAction::Hibernate => ("shutdown", vec!["/h"]),
        CompletionAction::Shutdown => ("shutdown", vec!["/s", "/t", "0"]),
        CompletionAction::Restart => ("shutdown", vec!["/r", "/t", "0"]),
        _ => return "failed: not a power action".into(),
    };
    run_process_status(program, &args)
}

#[cfg(not(windows))]
fn notify(queue_id: i64) -> String {
    let body = format!("Download queue {queue_id} completed");
    if which("notify-send").is_none() {
        return "failed: notify-send not found".into();
    }
    run_process_status("notify-send", &["-a", "XDM", "XDM Download Manager", &body])
}

/// Resolves a program name against PATH (standard directories when unset).
#[cfg(not(windows))]
fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .unwrap_or_else(|| "/usr/bin:/bin".into())
        .as_os_str()
        .to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(program))
        .find(|path| path.is_file())
}

#[cfg(not(windows))]
fn power_action(action: &CompletionAction) -> String {
    let (program, args): (&str, &[&str]) = match action {
        CompletionAction::Sleep => ("systemctl", &["suspend"]),
        CompletionAction::Hibernate => ("systemctl", &["hibernate"]),
        CompletionAction::Shutdown => ("systemctl", &["poweroff"]),
        CompletionAction::Restart => ("systemctl", &["reboot"]),
        _ => return "failed: not a power action".into(),
    };
    run_process_status(program, args)
}

/// Test adapter: appends one JSON object per executed action to a file in the
/// data dir, so tests can verify what would have run. Destructive actions are
/// only recorded when their countdown actually elapsed (armed); `Program` is
/// genuinely launched because argument passing is what the tests assert.
pub struct FakeOs {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FakeOs {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn record(&self, entry: serde_json::Value) {
        let _guard = self.lock.lock();
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{entry}");
        }
    }
}

impl CompletionAdapter for FakeOs {
    fn run(&self, queue_id: i64, action: &str, armed: bool) -> String {
        let Some(parsed) = CompletionAction::decode(action) else {
            return "failed: unknown action".into();
        };
        if parsed.is_destructive() && !armed {
            return "recorded".into();
        }
        let at = crate::unix_now();
        match parsed {
            CompletionAction::Program { program, args } => {
                let mut entry = serde_json::json!({
                    "at": at,
                    "queue_id": queue_id,
                    "action": "program",
                    "program": program,
                    "args": args,
                });
                let outcome = run_process_status(&program, &args.iter().map(String::as_str).collect::<Vec<_>>());
                if outcome == "executed" {
                    entry["exit"] = serde_json::json!(0);
                } else {
                    entry["exit"] = serde_json::Value::Null;
                    entry["error"] = serde_json::json!(outcome);
                }
                self.record(entry);
                outcome
            }
            other => {
                self.record(serde_json::json!({
                    "at": at,
                    "queue_id": queue_id,
                    "action": other.encode(),
                }));
                "recorded".into()
            }
        }
    }
}

/// Selects the adapter: tests force the fake through `XDM_FAKE_OS_ACTIONS=1`
/// so no real power action can ever run.
pub fn select(data_dir: &Path) -> Arc<dyn CompletionAdapter> {
    if std::env::var("XDM_FAKE_OS_ACTIONS").as_deref() == Ok("1") {
        Arc::new(FakeOs::new(data_dir.join("os_actions.json")))
    } else {
        Arc::new(RealOs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "xdm-os-actions-{}-{name}",
            std::process::id() as u64 ^ SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos() as u64
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn real_adapter_never_executes_unarmed_power_actions() {
        // Without the opted-in countdown the real adapter must degrade to
        // plain recording: spawning systemctl here would power off the box.
        for action in ["sleep", "hibernate", "shutdown", "restart"] {
            assert_eq!(RealOs.run(1, action, false), "recorded");
        }
    }

    #[test]
    fn fake_adapter_records_power_actions_to_jsonl() {
        let path = temp_file("shutdown.jsonl");
        let fake = FakeOs::new(path.clone());
        assert_eq!(fake.run(3, "shutdown", true), "recorded");
        assert_eq!(fake.run(3, "sleep", false), "recorded", "unarmed is not recorded");
        let lines = std::fs::read_to_string(&path).unwrap();
        let entry: serde_json::Value = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(
            (entry["action"].as_str(), entry["queue_id"].as_i64()),
            (Some("shutdown"), Some(3))
        );
        assert_eq!(lines.lines().count(), 1, "the unarmed sleep left no record");
        drop(fake);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn fake_adapter_program_launches_with_literal_arguments() {
        let file = temp_file("argv.txt");
        let script = format!("printf '%s\\n' \"$@\" > {}", file.display());
        let args = vec!["-c".into(), script, "xdm".into(), "hello world".into(), "a;b|>$HOME & ()".into()];
        let fake = FakeOs::new(temp_file("program.jsonl"));
        let status = fake.run(
            5,
            &CompletionAction::Program { program: "/bin/sh".into(), args: args.clone() }.encode(),
            false,
        );
        assert_eq!(status, "executed");
        let recorded = format!("hello world\na;b|>$HOME & ()\n");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), recorded);
        let lines = std::fs::read_to_string(fake.path.clone()).unwrap();
        let entry: serde_json::Value = serde_json::from_str(lines.trim()).unwrap();
        assert_eq!(entry["args"], serde_json::json!(args));
        assert_eq!(entry["exit"], serde_json::json!(0));
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(&fake.path);
    }

    #[test]
    fn fake_adapter_program_reports_failures() {
        let fake = FakeOs::new(temp_file("missing.jsonl"));
        let status = fake.run(
            7,
            &CompletionAction::Program {
                program: "/nonexistent-xdm-test/missing.bin".into(),
                args: Vec::new(),
            }
            .encode(),
            false,
        );
        assert!(status.starts_with("/nonexistent-xdm-test/missing.bin"));
        let _ = std::fs::remove_file(&fake.path);
    }
}
