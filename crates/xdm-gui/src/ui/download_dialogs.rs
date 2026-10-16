//! The download dialogs: "Download File Info", the per-download progress
//! window and "Download complete". Every daemon call runs on the IPC worker
//! thread (`ipc_then`); only property updates happen on the UI thread.

use crate::settings::{self, GuiSettings};
use crate::{IpcClient, dialog_helpers, fetch_task, ipc_then, task_table};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xdm_ipc::{Command, ImportEntry, Paths, SegmentInfo, TaskSummary};

slint::slint! {
    import { FileInfoWindow, ProgressWindow, CompleteWindow, SegView, ConnRow } from "download_dialogs.slint";
    export { FileInfoWindow, ProgressWindow, CompleteWindow, SegView, ConnRow }
}

/// Poll period of the progress window (about four updates per second).
const POLL: Duration = Duration::from_millis(250);
const HISTORY_LIMIT: usize = 10;

#[derive(Clone)]
pub struct DialogCtx {
    pub ipc: IpcClient,
    pub dir: PathBuf,
    pub settings: Arc<Mutex<GuiSettings>>,
    pub refresh: Arc<dyn Fn() + Send + Sync>,
    pub open_categories: Rc<dyn Fn()>,
}

struct Progress {
    window: ProgressWindow,
    _timer: Timer,
    finished: Rc<Cell<bool>>,
}

thread_local! {
    static FILE_INFO: RefCell<Vec<FileInfoWindow>> = const { RefCell::new(Vec::new()) };
    static COMPLETE: RefCell<Vec<CompleteWindow>> = const { RefCell::new(Vec::new()) };
    static PROGRESS: RefCell<HashMap<i64, Progress>> = RefCell::new(HashMap::new());
}

// ---------------------------------------------------------------- formatting

pub fn fmt_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} Bytes")
    } else if bytes < 1024 * 1024 {
        format!("{:.3} KB", b / KB)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", b / KB / KB)
    } else {
        format!("{:.2} GB", b / KB / KB / KB)
    }
}

pub fn fmt_rate(bytes_per_second: u64) -> String {
    if bytes_per_second < 1024 * 1024 {
        format!("{:.3} KB/sec", bytes_per_second as f64 / 1024.0)
    } else {
        format!("{:.2} MB/sec", bytes_per_second as f64 / 1024.0 / 1024.0)
    }
}

pub fn fmt_time(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} sec")
    } else if seconds < 3600 {
        format!("{} min {} sec", seconds / 60, seconds % 60)
    } else if seconds < 86_400 {
        format!("{} hour {} min", seconds / 3600, seconds % 3600 / 60)
    } else {
        format!("{} days {} hour", seconds / 86_400, seconds % 86_400 / 3600)
    }
}

fn percent_of(done: Option<u64>, total: Option<u64>) -> f64 {
    match (done, total) {
        (Some(done), Some(total)) if total > 0 => (done as f64 / total as f64 * 100.0).min(100.0),
        _ => 0.0,
    }
}

pub fn filename_from_url(url: &str) -> String {
    let name = url::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_owned))
        })
        .map(|segment| {
            percent_decode(&segment)
        })
        .unwrap_or_default();
    let clean: String = name
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let clean = clean.trim().trim_matches('.').to_owned();
    if clean.is_empty() { "download.bin".into() } else { clean }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let value = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(value) = value {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `name (2).ext`, `name (3).ext`, ... until the path does not exist.
pub fn numbered_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 2..10_000 {
        let candidate = path.with_file_name(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// Directory a category saves into: its rule's destination, else the
/// download root with a folder per category (`Other` stays in the root).
pub fn category_dir(
    rules: &[xdm_categories::CategoryRule],
    downloads: &Path,
    category: &str,
) -> PathBuf {
    if let Some(destination) = xdm_categories::destination_for(rules, category) {
        PathBuf::from(destination)
    } else if category == "Other" || category.is_empty() {
        downloads.to_path_buf()
    } else {
        downloads.join(category)
    }
}

/// Content length from a HEAD request (never a GET, so one-time links are not
/// consumed); `http://` only. Any failure means "unknown".
pub fn probe_size(url: &str) -> Option<u64> {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "http" || !parsed.username().is_empty() {
        return None;
    }
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    let address = (host, port).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
    let target = match parsed.query() {
        Some(query) => format!("{}?{query}", parsed.path()),
        None => parsed.path().to_owned(),
    };
    let host_header = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    write!(
        stream,
        "HEAD {target} HTTP/1.1\r\nHost: {host_header}\r\nAccept-Encoding: identity\r\nUser-Agent: XDM\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    while head.len() < 16 * 1024 && !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    parse_probe_head(&String::from_utf8_lossy(&head))
}

fn parse_probe_head(head: &str) -> Option<u64> {
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let header = |name: &str| {
        head.split("\r\n").skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_owned())
        })
    };
    match status {
        206 => header("content-range")?.rsplit('/').next()?.trim().parse().ok(),
        200 => header("content-length")?.parse().ok(),
        _ => None,
    }
}

// ------------------------------------------------------- OS helpers (no shell)

fn fake_os() -> bool {
    std::env::var("XDM_FAKE_OS_ACTIONS").as_deref() == Ok("1")
}

fn log_fake(line: &str) {
    if let Ok(dir) = std::env::var("XDM_DATA_DIR") {
        use std::io::Write;
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Path::new(&dir).join("gui-open.log"))
        {
            let _ = writeln!(log, "{line}");
        }
    }
}

fn run_capture(program: &str, args: &[&str], env: &[(&str, &str)]) -> Option<String> {
    let output = std::process::Command::new(program).args(args).envs(env.iter().copied()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Native file chooser through the platform's own dialog program.
fn choose_path(save: bool, initial: &Path) -> Option<PathBuf> {
    if fake_os() {
        log_fake(&format!("choose\t{}\t{}", if save { "save" } else { "program" }, initial.display()));
        return std::env::var("XDM_FAKE_CHOOSER_PATH").ok().map(PathBuf::from);
    }
    let initial_text = initial.to_string_lossy().into_owned();
    let picked = if cfg!(target_os = "windows") {
        let script = if save {
            "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.SaveFileDialog; $d.FileName = $env:XDM_INITIAL; if ($d.ShowDialog() -eq 'OK') { $d.FileName }"
        } else {
            "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; if ($d.ShowDialog() -eq 'OK') { $d.FileName }"
        };
        run_capture("powershell", &["-NoProfile", "-STA", "-Command", script], &[("XDM_INITIAL", &initial_text)])
    } else if cfg!(target_os = "macos") {
        let script = if save { "POSIX path of (choose file name)" } else { "POSIX path of (choose file)" };
        run_capture("osascript", &["-e", script], &[])
    } else {
        let filename = format!("--filename={initial_text}");
        let zenity: Vec<&str> = if save {
            vec!["--file-selection", "--save", "--confirm-overwrite", &filename]
        } else {
            vec!["--file-selection"]
        };
        run_capture("zenity", &zenity, &[]).or_else(|| {
            if save {
                run_capture("kdialog", &["--getsavefilename", &initial_text], &[])
            } else {
                run_capture("kdialog", &["--getopenfilename"], &[])
            }
        })
    };
    picked.map(PathBuf::from)
}

/// "Open with...": the user picks a program, which receives the file as an
/// argument vector entry (never a shell string).
fn open_with(output: &str) {
    let Some(program) = choose_path(false, Path::new(output)) else { return };
    if fake_os() {
        log_fake(&format!("{}\t{output}", program.display()));
        return;
    }
    let _ = std::process::Command::new(program).arg(output).spawn();
}

fn save_settings_now(ctx: &DialogCtx) {
    if let Ok(guard) = ctx.settings.lock() {
        let _ = settings::save_settings(&ctx.dir, &guard);
    }
}

/// Descriptions have no daemon column; they are kept beside the GUI settings.
fn remember_description(dir: &Path, task_id: i64, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let path = dir.join("gui-descriptions.json");
    let mut map: serde_json::Map<String, serde_json::Value> = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    map.insert(task_id.to_string(), serde_json::Value::String(text.trim().to_owned()));
    if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
        let _ = std::fs::write(path, bytes);
    }
}

// ------------------------------------------------------------ submit plumbing

#[derive(Clone)]
struct Plan {
    url: String,
    output: PathBuf,
    category: String,
    later: bool,
    allow_duplicate: bool,
    connections: usize,
    /// Directory to store as the category's destination.
    remember: Option<String>,
    description: String,
}

enum Outcome {
    Added(i64),
    Duplicate(i64),
    Rejected(String),
}

fn new_request_id() -> String {
    format!(
        "gui-add-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

/// Next numbered name: `a.bin` -> `a (2).bin`, `a (2).bin` -> `a (3).bin`.
pub fn bump_number(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let (base, next) = match stem.rsplit_once(" (").and_then(|(base, tail)| {
        tail.strip_suffix(')')?.parse::<u32>().ok().map(|n| (base.to_owned(), n + 1))
    }) {
        Some(pair) => pair,
        None => (stem, 2),
    };
    path.with_file_name(format!("{base} ({next}){ext}"))
}

/// Runs the plan; an explicit duplicate whose file name is already assigned
/// to a task moves on to the next numbered name instead of failing.
fn execute_plan(
    paths: &Paths,
    plan: &Plan,
    rules: &[xdm_categories::CategoryRule],
) -> (Outcome, PathBuf) {
    let mut plan = plan.clone();
    for _ in 0..50 {
        let outcome = execute_once(paths, &plan, rules);
        match &outcome {
            Outcome::Rejected(error) if plan.allow_duplicate && error.contains("assigned to a task") => {
                plan.output = numbered_path(&bump_number(&plan.output));
            }
            _ => return (outcome, plan.output),
        }
    }
    (Outcome::Rejected("No free numbered file name".into()), plan.output)
}

fn execute_once(paths: &Paths, plan: &Plan, rules: &[xdm_categories::CategoryRule]) -> Outcome {
    let reply = if plan.later {
        let Some(filename) = plan.output.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            return Outcome::Rejected("Save As needs a file name".into());
        };
        let directory = plan
            .output
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let category = if xdm_categories::CATEGORIES.contains(&plan.category.as_str()) {
            plan.category.clone()
        } else {
            "Other".into()
        };
        paths.request(Command::ImportListEx {
            request_id: new_request_id(),
            directory,
            entries: vec![ImportEntry {
                url: plan.url.clone(),
                filename,
                kind: "http".into(),
                priority: 1,
                connections: plan.connections.clamp(1, 16),
                rate_limit: None,
                video: None,
                audio: None,
                category,
                headers: Vec::new(),
            }],
            enqueue: false,
            allow_duplicate: plan.allow_duplicate,
        })
    } else {
        paths.request(Command::SubmitEx {
            request_id: new_request_id(),
            url: plan.url.clone(),
            output: plan.output.to_string_lossy().into_owned(),
            headers: Vec::new(),
            method: "GET".into(),
            body: None,
            priority: 1,
            connections: plan.connections,
            name_from_server: false,
            ignore_modification_date: false,
            allow_duplicate: plan.allow_duplicate,
            preserve_server_time: true,
        })
    };
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) => return Outcome::Rejected(format!("Daemon unavailable: {error}")),
    };
    if let Some(existing) = reply.duplicate_of {
        return Outcome::Duplicate(existing);
    }
    if let Some(skipped) = reply.skipped_duplicates.first() {
        return Outcome::Duplicate(skipped.duplicate_of.unwrap_or(0));
    }
    if !reply.ok {
        return Outcome::Rejected(reply.error.unwrap_or_else(|| "Download rejected".into()));
    }
    let Some(task_id) = reply.task_id.or_else(|| reply.task_ids.first().copied()) else {
        return Outcome::Rejected("The daemon accepted nothing".into());
    };
    if let Some(directory) = &plan.remember {
        let existing = rules.iter().find(|rule| rule.name == plan.category);
        let _ = paths.request(Command::CategorySave {
            name: plan.category.clone(),
            extensions: existing
                .map(|rule| rule.extensions.clone())
                .unwrap_or_else(|| xdm_categories::builtin_extensions(&plan.category)),
            destination: directory.clone(),
            sites: existing.map(|rule| rule.sites.clone()).unwrap_or_default(),
        });
    }
    Outcome::Added(task_id)
}

fn category_rules(paths: &Paths) -> Vec<xdm_categories::CategoryRule> {
    paths
        .request(Command::CategoryList)
        .ok()
        .filter(|reply| reply.ok)
        .map(|reply| reply.categories)
        .unwrap_or_default()
}

fn after_added(ctx: &DialogCtx, plan_later: bool, task_id: i64, output: &Path) {
    {
        if let Ok(mut guard) = ctx.settings.lock() {
            let text = output.to_string_lossy().into_owned();
            guard.save_history.retain(|entry| *entry != text);
            guard.save_history.insert(0, text);
            guard.save_history.truncate(HISTORY_LIMIT);
        }
        save_settings_now(ctx);
    }
    (ctx.refresh)();
    let show = ctx.settings.lock().map(|s| s.show_progress_window).unwrap_or(true);
    if !plan_later && show {
        open_progress(ctx, task_id);
    }
}

// ------------------------------------------------------------- File Info

/// Add URL entry point: the File Info dialog, or a direct start when the
/// "show download dialog" setting is off. `typed_destination` is a Save As
/// path the user typed in the Add URL dialog.
pub fn begin_add(ctx: &DialogCtx, url: String, typed_destination: Option<String>) {
    let show = ctx.settings.lock().map(|s| s.show_file_info).unwrap_or(true);
    if show {
        open_file_info(ctx, url, typed_destination);
        return;
    }
    let ctx = ctx.clone();
    let downloads = ctx.dir.join("downloads");
    let connections = ctx.settings.lock().map(|s| s.default_connections).unwrap_or(settings::DEFAULT_CONNECTIONS);
    let work_url = url.clone();
    let typed = typed_destination.clone();
    ipc_then(
        &ctx.ipc.clone(),
        move |paths| {
            let rules = category_rules(paths);
            let name = filename_from_url(&work_url);
            let host = url::Url::parse(&work_url).ok().and_then(|u| u.host_str().map(str::to_owned));
            let category = xdm_categories::classify_with(&rules, &name, host.as_deref(), None);
            let output = match typed {
                Some(path) => PathBuf::from(path),
                None => numbered_path(&category_dir(&rules, &downloads, &category).join(&name)),
            };
            let plan = Plan {
                url: work_url,
                output: output.clone(),
                category,
                later: false,
                allow_duplicate: false,
                connections,
                remember: None,
                description: String::new(),
            };
            execute_plan(paths, &plan, &rules)
        },
        move |(outcome, output)| match outcome {
            Outcome::Added(id) => after_added(&ctx, false, id, &output),
            // No window to ask in: fall back to the File Info dialog for the prompt.
            Outcome::Duplicate(_) | Outcome::Rejected(_) => open_file_info(&ctx, url, typed_destination),
        },
    );
}

struct InfoState {
    rules: Vec<xdm_categories::CategoryRule>,
    names: Vec<String>,
    downloads: PathBuf,
    category: String,
    filename: String,
    user_edited_path: bool,
    pending_later: bool,
    duplicate_of: i64,
}

fn parent_text(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|p| {
            let mut text = p.to_string_lossy().into_owned();
            if !text.is_empty() && !text.ends_with(std::path::MAIN_SEPARATOR) {
                text.push(std::path::MAIN_SEPARATOR);
            }
            text
        })
        .unwrap_or_default()
}

fn sync_remember(window: &FileInfoWindow, category: &str) {
    window.set_remember_label(format!("Remember this path for \"{category}\" category").into());
    window.set_remember_path(parent_text(window.get_save_as().as_str()).into());
}

fn apply_category_path(window: &FileInfoWindow, state: &InfoState) {
    if !state.user_edited_path {
        let path = numbered_path(&category_dir(&state.rules, &state.downloads, &state.category).join(&state.filename));
        window.set_save_as(path.to_string_lossy().as_ref().into());
    }
    sync_remember(window, &state.category);
}

fn open_file_info(ctx: &DialogCtx, url: String, typed_destination: Option<String>) {
    let Ok(window) = FileInfoWindow::new() else { return };
    let downloads = ctx.dir.join("downloads");
    let filename = filename_from_url(&url);
    // A path equal to the Add URL dialog's own default is not a user choice.
    let typed = typed_destination.filter(|path| {
        !path.trim().is_empty() && Path::new(path.trim()) != downloads.join(&filename)
    });
    let names: Vec<String> = xdm_categories::CATEGORIES.iter().map(|n| n.to_string()).collect();
    let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned));
    let category = xdm_categories::classify_with(&[], &filename, host.as_deref(), None);
    let state = Rc::new(RefCell::new(InfoState {
        rules: Vec::new(),
        names: names.clone(),
        downloads,
        category: category.clone(),
        filename,
        user_edited_path: typed.is_some(),
        pending_later: false,
        duplicate_of: 0,
    }));
    window.set_url(url.clone().into());
    window.set_categories(ModelRc::new(VecModel::from(
        names.iter().map(|n| SharedString::from(n.as_str())).collect::<Vec<_>>(),
    )));
    window.set_category_index(names.iter().position(|n| *n == category).unwrap_or(0) as i32);
    match &typed {
        Some(path) => window.set_save_as(path.as_str().into()),
        None => apply_category_path(&window, &state.borrow()),
    }
    sync_remember(&window, &category);
    let history = ctx.settings.lock().map(|s| s.save_history.clone()).unwrap_or_default();
    window.set_history(ModelRc::new(VecModel::from(
        history.iter().map(|h| SharedString::from(h.as_str())).collect::<Vec<_>>(),
    )));

    // Categories (with user rules) and the size probe come from worker threads.
    {
        let weak = window.as_weak();
        let state = state.clone();
        ipc_then(&ctx.ipc, |paths| category_rules(paths), move |rules| {
            let Some(window) = weak.upgrade() else { return };
            if rules.is_empty() {
                return;
            }
            let mut st = state.borrow_mut();
            let mut names: Vec<String> = rules.iter().map(|r| r.name.clone()).collect();
            for name in xdm_categories::CATEGORIES {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
            let current = window.get_category_index();
            let current_name = st.names.get(current as usize).cloned().unwrap_or_default();
            if !st.user_edited_path {
                let host = url::Url::parse(window.get_url().as_str()).ok().and_then(|u| u.host_str().map(str::to_owned));
                st.category = xdm_categories::classify_with(&rules, &st.filename, host.as_deref(), None);
            } else {
                st.category = current_name;
            }
            st.rules = rules;
            st.names = names.clone();
            window.set_categories(ModelRc::new(VecModel::from(
                names.iter().map(|n| SharedString::from(n.as_str())).collect::<Vec<_>>(),
            )));
            window.set_category_index(names.iter().position(|n| *n == st.category).unwrap_or(0) as i32);
            apply_category_path(&window, &st);
        });
    }
    spawn_probe(window.as_weak(), url.clone());

    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_category_picked(move |name| {
            let Some(window) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            st.category = name.to_string();
            apply_category_path(&window, &st);
        });
    }
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_save_as_edited(move |_| {
            if let Some(window) = weak.upgrade() {
                let mut st = state.borrow_mut();
                st.user_edited_path = true;
                let category = st.category.clone();
                drop(st);
                sync_remember(&window, &category);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_url_accepted(move |value| {
            let Some(window) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            st.filename = filename_from_url(value.as_str());
            apply_category_path(&window, &st);
            window.set_size_text("unknown".into());
            spawn_probe(weak.clone(), value.to_string());
        });
    }
    {
        let open_categories = ctx.open_categories.clone();
        window.on_new_category(move || open_categories());
    }
    {
        let weak = window.as_weak();
        window.on_browse(move || {
            let Some(window) = weak.upgrade() else { return };
            let initial = PathBuf::from(window.get_save_as().as_str());
            let weak = weak.clone();
            std::thread::spawn(move || {
                let picked = choose_path(true, &initial);
                let _ = slint::invoke_from_event_loop(move || {
                    if let (Some(window), Some(path)) = (weak.upgrade(), picked) {
                        window.set_save_as(path.to_string_lossy().as_ref().into());
                        window.invoke_save_as_edited(window.get_save_as());
                    }
                });
            });
        });
    }
    {
        let weak = window.as_weak();
        window.on_cancel(move || {
            if let Some(window) = weak.upgrade() {
                let _ = window.hide();
            }
        });
    }
    for later in [false, true] {
        let weak = window.as_weak();
        let state = state.clone();
        let ctx = ctx.clone();
        let handler = move || {
            if let Some(window) = weak.upgrade() {
                state.borrow_mut().pending_later = later;
                submit_from_window(&ctx, &window, &state, false);
            }
        };
        if later {
            window.on_later(handler);
        } else {
            window.on_start(handler);
        }
    }
    {
        let weak = window.as_weak();
        let state = state.clone();
        let ctx = ctx.clone();
        window.on_dup_copy(move || {
            let Some(window) = weak.upgrade() else { return };
            // Never overwrite: the copy gets the next free numbered name.
            let numbered = numbered_path(Path::new(window.get_save_as().as_str()));
            window.set_save_as(numbered.to_string_lossy().as_ref().into());
            window.set_dup_visible(false);
            submit_from_window(&ctx, &window, &state, true);
        });
    }
    {
        let weak = window.as_weak();
        window.on_dup_cancel(move || {
            if let Some(window) = weak.upgrade() {
                window.set_dup_visible(false);
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = state.clone();
        let ctx = ctx.clone();
        window.on_dup_open(move || {
            let id = state.borrow().duplicate_of;
            if let Some(window) = weak.upgrade() {
                let _ = window.hide();
            }
            open_task(&ctx, id);
        });
    }

    FILE_INFO.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows.retain(|w| w.window().is_visible());
        for old in windows.iter() {
            let _ = old.hide();
        }
        let _ = window.show();
        windows.push(window);
    });
}

fn spawn_probe(weak: slint::Weak<FileInfoWindow>, url: String) {
    std::thread::spawn(move || {
        let size = probe_size(&url);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = weak.upgrade() {
                // The URL may have been edited meanwhile; only fill a matching one.
                if window.get_url().as_str() == url {
                    window.set_size_text(size.map_or("unknown".to_owned(), fmt_size).into());
                }
            }
        });
    });
}

fn submit_from_window(
    ctx: &DialogCtx,
    window: &FileInfoWindow,
    state: &Rc<RefCell<InfoState>>,
    allow_duplicate: bool,
) {
    let url = match dialog_helpers::validate_download_url(window.get_url().as_str()) {
        Ok(url) => url,
        Err(error) => {
            window.set_message(error.into());
            return;
        }
    };
    let save_as = window.get_save_as().trim().to_owned();
    let mut output = PathBuf::from(&save_as);
    if save_as.is_empty() {
        window.set_message("Choose where to save the file".into());
        return;
    }
    if save_as.ends_with(['/', '\\']) || output.is_dir() {
        output = output.join(filename_from_url(&url));
    }
    if !output.is_absolute() {
        window.set_message("Save As must be a full path".into());
        return;
    }
    let st = state.borrow();
    let remember = window.get_remember().then(|| {
        output.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
    }).filter(|dir| {
        // Only a directory that differs from the category's current one is saved.
        !dir.is_empty()
            && Path::new(dir) != category_dir(&st.rules, &st.downloads, &st.category)
    });
    let plan = Plan {
        url,
        output: output.clone(),
        category: st.category.clone(),
        later: st.pending_later,
        allow_duplicate,
        connections: ctx.settings.lock().map(|s| s.default_connections).unwrap_or(settings::DEFAULT_CONNECTIONS),
        remember,
        description: window.get_description().to_string(),
    };
    let rules = st.rules.clone();
    drop(st);
    window.set_message("".into());
    window.set_busy(true);
    let weak = window.as_weak();
    let state = state.clone();
    let done_ctx = ctx.clone();
    let later = plan.later;
    let description = plan.description.clone();
    let data_dir = ctx.dir.clone();
    ipc_then(
        &ctx.ipc,
        move |paths| {
            let (outcome, output) = execute_plan(paths, &plan, &rules);
            if let Outcome::Added(id) = &outcome {
                remember_description(&data_dir, *id, &description);
            }
            (outcome, output)
        },
        move |(outcome, output)| {
            let window = weak.upgrade();
            if let Some(window) = &window {
                window.set_busy(false);
            }
            match outcome {
                Outcome::Added(id) => {
                    if let Some(window) = &window {
                        let _ = window.hide();
                    }
                    after_added(&done_ctx, later, id, &output);
                }
                Outcome::Duplicate(existing) => {
                    state.borrow_mut().duplicate_of = existing;
                    if let Some(window) = window {
                        window.set_dup_text(
                            format!(
                                "This address is already in the download list (task #{existing}). XDM never overwrites a file: add a numbered copy, open the existing download, or cancel."
                            )
                            .into(),
                        );
                        window.set_dup_visible(true);
                    }
                }
                Outcome::Rejected(error) => {
                    if let Some(window) = window {
                        window.set_message(error.into());
                    }
                }
            }
        },
    );
}

/// Opens the dialog that fits a task's state: complete dialog or progress window.
pub fn open_task(ctx: &DialogCtx, task_id: i64) {
    let ctx = ctx.clone();
    ipc_then(
        &ctx.ipc.clone(),
        move |paths| fetch_task(paths, task_id),
        move |result| {
            if let Ok(task) = result {
                if task_table::is_complete(&task.status) {
                    open_complete(&ctx, &task);
                } else {
                    open_progress(&ctx, task.id);
                }
            }
        },
    );
}

// --------------------------------------------------------- Download complete

pub fn open_complete(ctx: &DialogCtx, task: &TaskSummary) {
    let Ok(window) = CompleteWindow::new() else { return };
    let size = task.total.or(task.downloaded);
    window.set_size_line(
        size.map_or("Downloaded".to_owned(), |n| format!("Downloaded {} ({n} Bytes)", fmt_size(n))).into(),
    );
    window.set_address(task.url.as_str().into());
    window.set_saved_as(task.output.as_str().into());
    let output = task.output.clone();
    window.on_open({
        let output = output.clone();
        move || task_table::open_file(&output)
    });
    window.on_open_folder({
        let output = output.clone();
        move || task_table::open_folder(&output)
    });
    window.on_open_with({
        let output = output.clone();
        move || {
            let output = output.clone();
            std::thread::spawn(move || open_with(&output));
        }
    });
    let weak = window.as_weak();
    window.on_dismiss(move || {
        if let Some(window) = weak.upgrade() {
            let _ = window.hide();
        }
    });
    let ctx = ctx.clone();
    window.on_dont_show_toggled(move |dont| {
        if let Ok(mut guard) = ctx.settings.lock() {
            guard.show_complete_dialog = !dont;
        }
        save_settings_now(&ctx);
    });
    COMPLETE.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows.retain(|w| w.window().is_visible());
        let _ = window.show();
        windows.push(window);
    });
}

// ---------------------------------------------------------- Progress window

struct ProgressState {
    task_id: i64,
    status: String,
    in_flight: bool,
    limit_loaded: bool,
    empty_polls: u32,
    saw_segments: bool,
}

pub fn status_line(task: &TaskSummary) -> (String, bool) {
    let done = task.downloaded.unwrap_or(0);
    match task.status.as_str() {
        "running" if done == 0 => ("Connecting...".into(), false),
        "running" => ("Receiving data...".into(), false),
        "queued" if task.retry_at.is_some() => ("Waiting to retry...".into(), false),
        "queued" => ("Queued".into(), false),
        "paused" => ("Paused".into(), false),
        "pausing" => ("Pausing...".into(), false),
        "quota_wait" => ("Waiting for the bandwidth quota".into(), false),
        "auth_required" => ("Authentication required".into(), true),
        "restart_required" => ("Restart required".into(), true),
        "failed" => (task.error.clone().unwrap_or_else(|| "Download failed".into()), true),
        status if task_table::is_complete(status) => ("Download complete".into(), false),
        other => (other.to_owned(), false),
    }
}

/// Connections table rows: one per checkpoint segment, padded to eight rows.
pub fn connection_rows(
    segments: &[SegmentInfo],
    task: &TaskSummary,
) -> (Vec<(f32, f32, f32)>, Vec<(String, String, String)>) {
    let running = task.status == "running";
    let total = task
        .total
        .filter(|t| *t > 0)
        .or_else(|| segments.iter().map(|s| s.end.saturating_add(1)).max())
        .unwrap_or(0);
    let mut bars = Vec::new();
    let mut rows = Vec::new();
    if segments.is_empty() {
        if let Some(done) = task.downloaded.filter(|d| *d > 0) {
            let fraction = if total > 0 { (done as f64 / total as f64).min(1.0) as f32 } else { 0.0 };
            bars.push((0.0, fraction, 1.0));
            let info = if running { "Receiving data..." } else { "" };
            rows.push(("1".to_owned(), fmt_size(done), info.to_owned()));
        }
    } else {
        for (index, seg) in segments.iter().enumerate() {
            let scale = |value: u64| if total > 0 { (value as f64 / total as f64).min(1.0) as f32 } else { 0.0 };
            bars.push((scale(seg.start), scale(seg.cursor), scale(seg.end.saturating_add(1))));
            let finished = seg.cursor > seg.end;
            let info = if finished {
                "Download complete"
            } else if running {
                "Receiving data..."
            } else {
                ""
            };
            rows.push(((index + 1).to_string(), fmt_size(seg.cursor.saturating_sub(seg.start)), info.to_owned()));
        }
    }
    while rows.len() < 8 {
        rows.push(((rows.len() + 1).to_string(), String::new(), String::new()));
    }
    (bars, rows)
}

pub fn open_progress(ctx: &DialogCtx, task_id: i64) {
    let existing = PROGRESS.with(|map| {
        let mut map = map.borrow_mut();
        map.retain(|_, entry| !entry.finished.get());
        map.get(&task_id).map(|entry| {
            let _ = entry.window.show();
        })
    });
    if existing.is_some() {
        return;
    }
    let Ok(window) = ProgressWindow::new() else { return };
    let show_complete = ctx.settings.lock().map(|s| s.show_complete_dialog).unwrap_or(true);
    window.set_opt_show_complete(show_complete);
    window.set_title_text("Download".into());
    window.set_status_text("Connecting...".into());
    window.set_resume_text("unknown".into());
    window.set_file_size_text("unknown".into());
    let state = Rc::new(RefCell::new(ProgressState {
        task_id,
        status: String::new(),
        in_flight: false,
        limit_loaded: false,
        empty_polls: 0,
        saw_segments: false,
    }));
    let finished = Rc::new(Cell::new(false));

    {
        let weak = window.as_weak();
        let state = state.clone();
        let ctx = ctx.clone();
        window.on_pause_resume(move || {
            let st = state.borrow();
            let command = match st.status.as_str() {
                "paused" | "failed" | "restart_required" | "auth_required" => Command::Resume { task_id: st.task_id },
                _ => Command::Pause { task_id: st.task_id },
            };
            drop(st);
            let weak = weak.clone();
            ipc_then(&ctx.ipc, move |paths| paths.request(command).map_err(|e| e.to_string()), move |result| {
                if let (Some(window), Ok(reply)) = (weak.upgrade(), result) {
                    if !reply.ok {
                        window.set_status_text(reply.error.unwrap_or_else(|| "Request rejected".into()).into());
                        window.set_status_error(true);
                    }
                }
            });
        });
    }
    {
        let weak = window.as_weak();
        let state = state.clone();
        let ctx = ctx.clone();
        let finished = finished.clone();
        window.on_cancel(move || {
            finished.set(true);
            let id = state.borrow().task_id;
            let status = state.borrow().status.clone();
            if !task_table::is_complete(&status) {
                ipc_then(&ctx.ipc, move |paths| { let _ = paths.request(Command::Pause { task_id: id }); }, |_| {});
            }
            if let Some(window) = weak.upgrade() {
                let _ = window.hide();
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = state.clone();
        let ctx = ctx.clone();
        window.on_limit_apply(move || {
            let Some(window) = weak.upgrade() else { return };
            let id = state.borrow().task_id;
            let digits: String = window.get_limit_kb().chars().filter(char::is_ascii_digit).collect();
            let rate = match (window.get_use_limit(), digits.parse::<u64>()) {
                (true, Ok(kb)) if kb > 0 => Some(kb.saturating_mul(1024)),
                _ => None,
            };
            if window.get_use_limit() && rate.is_none() {
                window.set_status_text("Enter the maximum speed in KBytes/sec".into());
                window.set_status_error(true);
                return;
            }
            let weak = weak.clone();
            ipc_then(&ctx.ipc, move |paths| {
                paths.request(Command::SetTaskRate { task_id: id, bytes_per_second: rate }).map_err(|e| e.to_string())
            }, move |result| {
                if let Some(window) = weak.upgrade() {
                    match result {
                        Ok(reply) if reply.ok => window.set_status_error(false),
                        Ok(reply) => {
                            window.set_status_text(reply.error.unwrap_or_else(|| "Speed limit rejected".into()).into());
                            window.set_status_error(true);
                        }
                        Err(error) => {
                            window.set_status_text(error.into());
                            window.set_status_error(true);
                        }
                    }
                }
            });
        });
    }

    let timer = Timer::default();
    {
        let weak = window.as_weak();
        let state = state.clone();
        let finished = finished.clone();
        let ctx = ctx.clone();
        timer.start(TimerMode::Repeated, POLL, move || {
            tick(&ctx, &weak, &state, &finished);
        });
    }
    let _ = window.show();
    tick(ctx, &window.as_weak(), &state, &finished);
    PROGRESS.with(|map| {
        map.borrow_mut().insert(task_id, Progress { window, _timer: timer, finished });
    });
}

fn tick(
    ctx: &DialogCtx,
    weak: &slint::Weak<ProgressWindow>,
    state: &Rc<RefCell<ProgressState>>,
    finished: &Rc<Cell<bool>>,
) {
    if finished.get() {
        return;
    }
    let Some(window) = weak.upgrade() else { return };
    if state.borrow().in_flight {
        return;
    }
    state.borrow_mut().in_flight = true;
    let task_id = state.borrow().task_id;
    let want_segments = window.get_details_visible();
    let weak = weak.clone();
    let state = state.clone();
    let finished = finished.clone();
    let ctx = ctx.clone();
    let ctx_work = ctx.clone();
    ipc_then(
        &ctx_work.ipc,
        move |paths| {
            let task = fetch_task(paths, task_id);
            let segments = want_segments
                .then(|| paths.request(Command::TaskSegments { task_id }).ok())
                .flatten()
                .filter(|reply| reply.ok)
                .map(|reply| reply.segments);
            (task, segments)
        },
        move |(task, segments)| {
            state.borrow_mut().in_flight = false;
            let Some(window) = weak.upgrade() else { return };
            match task {
                Ok(task) => apply_task(&ctx, &window, &state, &finished, &task, segments),
                Err(error) => {
                    window.set_status_text(error.into());
                    window.set_status_error(true);
                }
            }
        },
    );
}

fn apply_task(
    ctx: &DialogCtx,
    window: &ProgressWindow,
    state: &Rc<RefCell<ProgressState>>,
    finished: &Rc<Cell<bool>>,
    task: &TaskSummary,
    segments: Option<Vec<SegmentInfo>>,
) {
    let name = task_table::format_filename(&task.output, &task.url);
    let complete = task_table::is_complete(&task.status);
    let (done, total) = if complete {
        let size = task.total.or(task.downloaded);
        (size, size)
    } else {
        (task.downloaded, task.total)
    };
    let percent = percent_of(done, total);
    window.set_title_text(if complete { name.clone() } else { format!("{percent:.2}% {name}") }.into());
    window.set_url_text(task.url.as_str().into());
    let (status, error) = status_line(task);
    window.set_status_text(status.into());
    window.set_status_error(error);
    window.set_file_size_text(total.map_or("unknown".to_owned(), fmt_size).into());
    window.set_downloaded_text(
        done.map_or(String::new(), |d| format!("{} ( {percent:.2} % )", fmt_size(d))).into(),
    );
    let running = task.status == "running";
    window.set_rate_text(
        task.bytes_per_second.filter(|_| running).map_or(String::new(), fmt_rate).into(),
    );
    window.set_limiter_rate_text(
        task.bytes_per_second.filter(|_| running).map_or(String::new(), fmt_rate).into(),
    );
    window.set_time_left_text(task.eta_seconds.filter(|_| running).map_or(String::new(), fmt_time).into());
    window.set_percent((percent / 100.0) as f32);
    let resumable_pause = matches!(task.status.as_str(), "paused" | "failed" | "restart_required" | "auth_required");
    window.set_pause_text(if resumable_pause { "Resume" } else { "Pause" }.into());
    window.set_pause_enabled(!complete && task.status != "pausing");

    let mut st = state.borrow_mut();
    st.status = task.status.clone();
    if !st.limit_loaded {
        st.limit_loaded = true;
        if let Some(rate) = task.rate_limit {
            window.set_use_limit(true);
            window.set_limit_kb((rate / 1024).to_string().into());
        }
    }
    if let Some(segments) = segments {
        if segments.is_empty() {
            st.empty_polls += 1;
        } else {
            st.saw_segments = true;
        }
        let resume = if st.saw_segments {
            "Yes"
        } else if st.empty_polls >= 8 && done.unwrap_or(0) > 0 {
            "No"
        } else {
            "unknown"
        };
        window.set_resume_text(resume.into());
        let (bars, rows) = connection_rows(&segments, task);
        window.set_segments(ModelRc::new(VecModel::from(
            bars.into_iter().map(|(from, done, to)| SegView { from, done, to }).collect::<Vec<_>>(),
        )));
        window.set_connections(ModelRc::new(VecModel::from(
            rows.into_iter()
                .map(|(n, downloaded, info)| ConnRow {
                    n: n.into(),
                    downloaded: downloaded.into(),
                    info: info.into(),
                })
                .collect::<Vec<_>>(),
        )));
    }
    drop(st);

    let hidden = !window.window().is_visible();
    if complete {
        finished.set(true);
        let open_file = window.get_opt_open_file();
        let open_folder = window.get_opt_open_folder();
        let show_complete = window.get_opt_show_complete();
        let _ = window.hide();
        if open_file {
            task_table::open_file(&task.output);
        }
        if open_folder {
            task_table::open_folder(&task.output);
        }
        if show_complete {
            open_complete(ctx, task);
        }
        (ctx.refresh)();
    } else if hidden && !matches!(task.status.as_str(), "running" | "queued" | "pausing" | "quota_wait") {
        // Closed by the user and nothing left to watch.
        finished.set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: &str, downloaded: u64, total: u64) -> TaskSummary {
        serde_json::from_value(serde_json::json!({
            "id": 1, "url": "http://x/f.bin", "output": "/tmp/f.bin", "status": status,
            "priority": 1, "error": null, "downloaded": downloaded, "total": total
        }))
        .unwrap()
    }

    #[test]
    fn sizes_rates_and_times_follow_the_idm_style() {
        assert_eq!(fmt_size(512), "512 Bytes");
        assert_eq!(fmt_size(128 * 1024), "128.000 KB");
        assert_eq!(fmt_size(64 * 1024 * 1024), "64.00 MB");
        assert_eq!(fmt_rate(600_000), "585.938 KB/sec");
        assert_eq!(fmt_time(111), "1 min 51 sec");
        assert_eq!(fmt_time(7), "7 sec");
        assert_eq!(fmt_time(3700), "1 hour 1 min");
    }

    #[test]
    fn file_names_come_from_the_url_path() {
        assert_eq!(filename_from_url("http://h/a/b%20c.zip?x=1"), "b c.zip");
        assert_eq!(filename_from_url("http://h/"), "download.bin");
        assert_eq!(filename_from_url("http://h/a%2Fb.bin"), "a_b.bin");
        assert_eq!(filename_from_url("http://h/100%"), "100%");
    }

    #[test]
    fn numbers_increase() {
        assert_eq!(bump_number(Path::new("/d/a.bin")), PathBuf::from("/d/a (2).bin"));
        assert_eq!(bump_number(Path::new("/d/a (2).bin")), PathBuf::from("/d/a (3).bin"));
        assert_eq!(bump_number(Path::new("/d/a (x).bin")), PathBuf::from("/d/a (x) (2).bin"));
    }

    #[test]
    fn numbered_copies_never_reuse_an_existing_path() {
        let dir = std::env::temp_dir().join(format!("xdm-numbered-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("a.bin");
        assert_eq!(numbered_path(&base), base);
        std::fs::write(&base, b"x").unwrap();
        assert_eq!(numbered_path(&base), dir.join("a (2).bin"));
        std::fs::write(dir.join("a (2).bin"), b"x").unwrap();
        assert_eq!(numbered_path(&base), dir.join("a (3).bin"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn category_directory_uses_rule_destination_or_default_layout() {
        let rules = vec![xdm_categories::CategoryRule {
            name: "Video".into(),
            extensions: vec![],
            destination: "/srv/videos".into(),
            sites: vec![],
        }];
        let root = Path::new("/d");
        assert_eq!(category_dir(&rules, root, "Video"), PathBuf::from("/srv/videos"));
        assert_eq!(category_dir(&rules, root, "Music"), PathBuf::from("/d/Music"));
        assert_eq!(category_dir(&rules, root, "Other"), PathBuf::from("/d"));
    }

    #[test]
    fn the_size_probe_sends_head_and_never_get() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut methods = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 1024];
                let read = stream.read(&mut buffer).unwrap();
                methods.push(String::from_utf8_lossy(&buffer[..read]).split(' ').next().unwrap().to_owned());
                let reply: &[u8] = if methods.len() == 1 {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1234\r\n\r\n"
                } else {
                    b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n"
                };
                stream.write_all(reply).unwrap();
            }
            methods
        });
        assert_eq!(probe_size(&format!("http://127.0.0.1:{port}/a")), Some(1234));
        assert_eq!(probe_size(&format!("http://127.0.0.1:{port}/b")), None);
        assert_eq!(server.join().unwrap(), vec!["HEAD", "HEAD"]);
    }

    #[test]
    fn probe_replies_give_the_size() {
        assert_eq!(parse_probe_head("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/4096\r\n\r\n"), Some(4096));
        assert_eq!(parse_probe_head("HTTP/1.0 200 OK\r\ncontent-length: 77\r\n\r\n"), Some(77));
        assert_eq!(parse_probe_head("HTTP/1.1 404 Not Found\r\nContent-Length: 3\r\n\r\n"), None);
    }

    #[test]
    fn connection_rows_come_from_checkpoint_segments() {
        let segments = [
            SegmentInfo { start: 0, end: 99, cursor: 100 },
            SegmentInfo { start: 100, end: 199, cursor: 150 },
        ];
        let (bars, rows) = connection_rows(&segments, &task("running", 150, 200));
        assert_eq!(bars, vec![(0.0, 0.5, 0.5), (0.5, 0.75, 1.0)]);
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[0], ("1".into(), "100 Bytes".into(), "Download complete".into()));
        assert_eq!(rows[1], ("2".into(), "50 Bytes".into(), "Receiving data...".into()));
        assert_eq!(rows[2].1, "");
        // Without a checkpoint a running download is a single connection.
        let (bars, rows) = connection_rows(&[], &task("running", 50, 200));
        assert_eq!(bars, vec![(0.0, 0.25, 1.0)]);
        assert_eq!(rows[0].1, "50 Bytes");
    }

    #[test]
    fn status_lines_describe_the_task_state() {
        assert_eq!(status_line(&task("running", 0, 10)).0, "Connecting...");
        assert_eq!(status_line(&task("running", 5, 10)).0, "Receiving data...");
        assert_eq!(status_line(&task("paused", 5, 10)).0, "Paused");
        assert!(status_line(&task("failed", 5, 10)).1);
        assert_eq!(status_line(&task("complete", 10, 10)).0, "Download complete");
        assert!((percent_of(Some(1), Some(4)) - 25.0).abs() < 1e-9);
    }
}
