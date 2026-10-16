//! Glue for the IDM-style main window: menus, category tree, toolbar states,
//! column layout and persistence. Everything here runs on the UI thread; IPC
//! goes through `IpcClient` so the window never blocks on the daemon.
use crate::layout::{self, Layout};
use crate::*;
use slint::{LogicalSize, PhysicalPosition};
use std::cell::Cell;
use std::collections::HashSet;

/// Everything the shell needs from `main`.
pub struct Ctx {
    pub ipc: IpcClient,
    pub refresh: Arc<dyn Fn() + Send + Sync>,
    pub shared: TableShared,
    pub dir: std::path::PathBuf,
    pub sort_state: Arc<Mutex<(String, bool)>>,
}

#[derive(Clone, Default)]
struct QueueInfo {
    id: i64,
    name: String,
    running: bool,
}

#[derive(Clone, Copy, Default)]
struct Stats {
    active: usize,
    complete: usize,
    master_running: bool,
    any_queue_running: bool,
}

struct Shell {
    ctx: Ctx,
    layout: RefCell<Layout>,
    saved: RefCell<Layout>,
    tree: Rc<VecModel<TreeNode>>,
    cols: Rc<VecModel<ColumnSpec>>,
    cats: RefCell<Vec<String>>,
    queues: RefCell<Vec<QueueInfo>>,
    node: RefCell<String>,
    expanded: RefCell<HashSet<String>>,
    sub_open: RefCell<Option<String>>,
    stats: Cell<Stats>,
    find: RefCell<Option<FindWindow>>,
    prompt: RefCell<Option<PromptWindow>>,
    confirm: RefCell<Option<ConfirmWindow>>,
    about: RefCell<Option<AboutWindow>>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

const BUILTIN: [&str; 6] = ["Compressed", "Documents", "Music", "Programs", "Video", "Other"];

// ---- menu model helpers ---------------------------------------------------

struct E {
    label: String,
    shortcut: &'static str,
    action: String,
    enabled: bool,
    sep: bool,
    checked: bool,
    sub: bool,
}

fn it(label: &str, shortcut: &'static str, action: &str, enabled: bool) -> E {
    E { label: i18n::translate(label), shortcut, action: action.into(), enabled, sep: false, checked: false, sub: false }
}

fn raw(label: String, action: String, enabled: bool, checked: bool) -> E {
    E { label, shortcut: "", action, enabled, sep: false, checked, sub: false }
}

fn sub(label: &str, action: &str, enabled: bool) -> E {
    E { sub: true, ..it(label, "", action, enabled) }
}

fn sep() -> E {
    E { label: String::new(), shortcut: "", action: String::new(), enabled: false, sep: true, checked: false, sub: false }
}

fn checked(mut e: E, on: bool) -> E {
    e.checked = on;
    e
}

fn model(entries: Vec<E>) -> (ModelRc<MenuEntry>, f32) {
    let mut y = 2.0;
    let rows: Vec<MenuEntry> = entries
        .into_iter()
        .map(|e| {
            let row = MenuEntry {
                label: e.label.into(),
                shortcut: e.shortcut.into(),
                action: e.action.into(),
                enabled: e.enabled,
                separator: e.sep,
                checked: e.checked,
                submenu: e.sub,
                y,
            };
            y += if e.sep { 9.0 } else { 17.0 };
            row
        })
        .collect();
    (ModelRc::new(VecModel::from(rows)), y + 2.0)
}

// ---- selection ------------------------------------------------------------

struct Sel {
    ids: Vec<i64>,
    statuses: Vec<String>,
}

impl Sel {
    fn any(&self, f: impl Fn(&str) -> bool) -> bool {
        self.statuses.iter().any(|s| f(s))
    }
    fn single_complete(&self) -> bool {
        self.ids.len() == 1 && self.statuses.first().is_some_and(|s| task_table::is_complete(s))
    }
}

fn selection(ui: &MainWindow, shell: &Shell) -> Sel {
    let shared = &shell.ctx.shared;
    let ids = target_ids(ui, &shared.ids, &shared.selected);
    let statuses = ids.iter().filter_map(|id| shared.status_of(*id)).collect();
    Sel { ids, statuses }
}

fn resumable(s: &str) -> bool {
    matches!(s, "paused" | "failed")
}

fn stoppable(s: &str) -> bool {
    matches!(s, "queued" | "running" | "quota_wait")
}

// ---- menus ------------------------------------------------------------------

fn visible_flag(ui: &MainWindow, id: &str) -> bool {
    match id {
        "filename" => ui.get_col_filename(),
        "size" => ui.get_col_size(),
        "status" => ui.get_col_status(),
        "time_left" => ui.get_col_time_left(),
        "transfer_rate" => ui.get_col_transfer_rate(),
        "last_try_date" => ui.get_col_last_try_date(),
        "description" => ui.get_col_description(),
        "category" => ui.get_col_category(),
        "queue" => ui.get_col_queue(),
        _ => true,
    }
}

fn queue_entries(shell: &Shell, prefix: &str, only_running: Option<bool>) -> Vec<E> {
    let queues = shell.queues.borrow();
    let mut out: Vec<E> = queues
        .iter()
        .filter(|q| only_running.is_none_or(|r| q.running == r))
        .map(|q| raw(q.name.clone(), format!("{prefix}:{}", q.id), true, false))
        .collect();
    if out.is_empty() {
        out.push(it("(no queues)", "", "", false));
    }
    out
}

fn fill_menu(ui: &MainWindow, shell: &Shell, name: &str) {
    let sel = selection(ui, shell);
    let one = sel.ids.len() == 1;
    let any = !sel.ids.is_empty();
    let stats = shell.stats.get();
    match name {
        "tasks" => {
            let (m, h) = model(vec![
                it("Add new download", "", "add-url", true),
                it("Add batch download…", "", "add-batch", true),
                it("Add media download…", "", "add-media", true),
                it("Run site grabber", "", "grabber", true),
                sep(),
                it("Import / Export…", "", "import-export", true),
                sep(),
                it("Exit", "", "exit", true),
            ]);
            ui.set_m_tasks(m);
            ui.set_h_tasks(h);
        }
        "file" => {
            let (m, h) = model(vec![
                it("Download now", "", "download-now", sel.any(resumable)),
                it("Stop download", "", "stop", sel.any(stoppable)),
                it("Remove", "Del", "remove", any),
                it("Redownload", "", "redownload", one),
                it("Restart", "", "restart", one),
                sep(),
                it("Open", "", "open", sel.single_complete()),
                it("Open folder", "", "open-folder", one),
                it("Move/Rename", "Ctrl+M", "move-rename", sel.single_complete()),
                sep(),
                sub("Priority", "sub:priority", any),
                sub("Queue order", "sub:queueorder", one),
                sep(),
                it("Properties", "", "properties", one),
            ]);
            ui.set_m_file(m);
            ui.set_h_file(h);
        }
        "downloads" => {
            let (m, h) = model(vec![
                it("Resume all", "", "start-all", true),
                it("Stop all", "", "stop-all", stats.active > 0 || stats.master_running),
                it("Delete all completed", "", "delete-completed", stats.complete > 0),
                sep(),
                it("Find…", "Ctrl+F", "find", true),
                sep(),
                it("Scheduler", "", "scheduler", true),
                it("Queues…", "", "queues", true),
                sub("Start queue", "sub:startq", true),
                sub("Stop queue", "sub:stopq", true),
                it("Schedule queue start…", "", "schedule-start", true),
                it("Schedule queue stop…", "", "schedule-stop", true),
                sep(),
                sub("Speed Limiter", "sub:speed", any),
                it("Clean up partial files…", "", "partials", true),
                sep(),
                it("Categories…", "", "categories", true),
                it("Connection…", "", "connection", true),
                it("Options", "", "options", true),
            ]);
            ui.set_m_downloads(m);
            ui.set_h_downloads(h);
        }
        "view" => {
            let layout = shell.layout.borrow();
            let (m, h) = model(vec![
                checked(it("Hide categories", "", "toggle-sidebar", true), !layout.show_sidebar),
                sep(),
                sub("Arrange files", "sub:arrange", true),
                it("Columns…", "", "columns", true),
                sep(),
                checked(it("Large buttons", "", "toolbar-large", true), !layout.small_toolbar),
                checked(it("Small buttons", "", "toolbar-small", true), layout.small_toolbar),
                sep(),
                it("Refresh", "", "refresh", true),
            ]);
            ui.set_m_view(m);
            ui.set_h_view(h);
        }
        "help" => {
            let (m, h) = model(vec![it("About XDM", "", "about", true)]);
            ui.set_m_help(m);
            ui.set_h_help(h);
        }
        "startq" => {
            let (m, h) = model(queue_entries(shell, "startq", None));
            ui.set_m_startq(m);
            ui.set_h_startq(h);
        }
        "stopq" => {
            let (m, h) = model(queue_entries(shell, "stopq", None));
            ui.set_m_stopq(m);
            ui.set_h_stopq(h);
        }
        _ => {}
    }
}

fn fill_context(ui: &MainWindow, shell: &Shell) {
    let sel = selection(ui, shell);
    let one = sel.ids.len() == 1;
    let any = !sel.ids.is_empty();
    let dbl = shell.layout.borrow().double_click.clone();
    let _ = dbl;
    let (m, h) = model(vec![
        it("Open", "", "open", sel.single_complete()),
        it("Open folder", "", "open-folder", one),
        sep(),
        it("Move/Rename (Ctrl-M)", "", "move-rename", sel.single_complete()),
        sep(),
        it("Redownload", "", "redownload", one),
        sep(),
        it("Resume Download", "", "download-now", sel.any(resumable)),
        it("Stop Download", "", "stop", sel.any(stoppable)),
        it("Restart", "", "restart", one),
        sep(),
        it("Remove", "", "remove", any),
        sep(),
        sub("Add to queue", "sub:addq", any),
        it("Delete from queue", "", "delq", any),
        sep(),
        sub("On Double click", "sub:dblclick", true),
        sep(),
        it("Properties", "", "properties", one),
    ]);
    ui.set_m_context(m);
    ui.set_h_context(h);
}

fn fill_sub(ui: &MainWindow, shell: &Shell, name: &str) {
    let dbl = shell.layout.borrow().double_click.clone();
    let sort = shell.ctx.sort_state.lock().map(|s| s.0.clone()).unwrap_or_default();
    let entries = match name {
        "sub:priority" => vec![
            it("Immediate", "", "prio:2", true),
            it("Normal", "", "prio:1", true),
            it("Low", "", "prio:0", true),
        ],
        "sub:queueorder" => vec![
            it("Move to top", "", "move:top", true),
            it("Move up", "", "move:up", true),
            it("Move down", "", "move:down", true),
            it("Move to bottom", "", "move:bottom", true),
        ],
        "sub:speed" => vec![
            it("Turn on…", "", "speed:on", true),
            it("Turn off", "", "speed:off", true),
            it("Settings…", "", "speed:settings", true),
        ],
        "sub:startq" => queue_entries(shell, "startq", None),
        "sub:stopq" => queue_entries(shell, "stopq", None),
        "sub:addq" => queue_entries(shell, "addq", None),
        "sub:arrange" => [
            ("By Order Of Addition", "id"),
            ("By File Name", "filename"),
            ("By Size", "size"),
            ("By Status", "status"),
            ("By Time Left", "time_left"),
            ("By Transfer Rate", "transfer_rate"),
            ("By Last Try Date", "last_try_date"),
            ("By Description", "description"),
        ]
        .into_iter()
        .map(|(label, key)| checked(it(label, "", &format!("sort:{key}"), true), sort == key))
        .collect(),
        "sub:dblclick" => vec![
            checked(it("Open file", "", "dbl:open", true), dbl == "open"),
            checked(it("Open folder", "", "dbl:folder", true), dbl == "folder"),
            checked(it("Properties", "", "dbl:properties", true), dbl == "properties"),
        ],
        _ => return,
    };
    let (m, h) = model(entries);
    ui.set_m_sub(m);
    ui.set_h_sub(h);
}

// ---- tree -------------------------------------------------------------------

fn node(id: &str, label: &str, depth: i32, icon: &str, kids: bool, expanded: bool, selected: bool) -> TreeNode {
    TreeNode {
        id: id.into(),
        label: label.into(),
        depth,
        icon: icon.into(),
        expandable: kids,
        expanded: kids && expanded,
        selected,
    }
}

fn cat_icon(name: &str) -> &'static str {
    match name {
        "Compressed" => "cat-compressed",
        "Documents" => "cat-documents",
        "Music" => "cat-music",
        "Programs" => "cat-programs",
        "Video" => "cat-video",
        _ => "cat-other",
    }
}

fn rebuild_tree(shell: &Shell) {
    let expanded = shell.expanded.borrow();
    let current = shell.node.borrow().clone();
    let cats = shell.cats.borrow();
    let queues = shell.queues.borrow();
    let mut rows = Vec::new();
    let group = |rows: &mut Vec<TreeNode>, id: &str, label: &str, icon: &str| {
        let open = expanded.contains(id);
        let icon = if open && icon == "folder" { "folder-open" } else { icon };
        rows.push(node(id, &i18n::translate(label), 0, icon, true, open, current == id));
        if open {
            for name in cats.iter() {
                let child = if id == "all" { format!("cat:{name}") } else { format!("{id}:{name}") };
                rows.push(node(&child, &i18n::translate(name), 1, cat_icon(name), false, false, current == child));
            }
        }
    };
    group(&mut rows, "all", "All Downloads", "folder");
    group(&mut rows, "unfinished", "Unfinished", "unfinished");
    group(&mut rows, "finished", "Finished", "finished");
    rows.push(node("grabber", &i18n::translate("Grabber projects"), 0, "grabber", false, false, current == "grabber"));
    let open = expanded.contains("queues");
    rows.push(node("queues", &i18n::translate("Queues"), 0, "queues", true, open, current == "queues"));
    if open {
        for q in queues.iter() {
            let id = format!("queue:{}", q.id);
            rows.push(node(&id, &q.name, 1, "folder", false, false, current == id));
        }
    }
    shell.tree.set_vec(rows);
}

/// Pushes the selected tree node into the window filters.
fn apply_node(ui: &MainWindow, shell: &Shell, id: &str) {
    *shell.node.borrow_mut() = id.to_string();
    let (status, category, queue) = match id {
        "all" | "queues" => ("", "All".to_string(), 0i64),
        "unfinished" | "finished" => (id, "All".to_string(), 0),
        other => {
            if let Some(name) = other.strip_prefix("cat:") {
                ("", name.to_string(), 0)
            } else if let Some(q) = other.strip_prefix("queue:") {
                ("", "All".to_string(), q.parse().unwrap_or(0))
            } else if let Some(name) = other.strip_prefix("unfinished:") {
                ("unfinished", name.to_string(), 0)
            } else if let Some(name) = other.strip_prefix("finished:") {
                ("finished", name.to_string(), 0)
            } else {
                ("", "All".to_string(), 0)
            }
        }
    };
    ui.set_status_filter(status.into());
    ui.set_category_filter(category.into());
    ui.set_queue_filter_id(queue.to_string().into());
    ui.set_selected(-1);
    rebuild_tree(shell);
    (shell.ctx.refresh)();
}

// ---- toolbar states ---------------------------------------------------------

/// The Queues dialog can change the queue filter behind the tree's back; keep the highlight honest.
fn sync_node(ui: &MainWindow, shell: &Shell) {
    let queue = ui.get_queue_filter_id().to_string();
    let current = shell.node.borrow().clone();
    let wanted = match (queue.as_str(), current.strip_prefix("queue:")) {
        ("0", None) => return,
        ("0", Some(_)) => "all".to_string(),
        (q, Some(c)) if q == c => return,
        (q, _) => format!("queue:{q}"),
    };
    *shell.node.borrow_mut() = wanted;
    shell.expanded.borrow_mut().insert("queues".into());
    rebuild_tree(shell);
}

fn update_toolbar(ui: &MainWindow, shell: &Shell) {
    sync_node(ui, shell);
    let sel = selection(ui, shell);
    let stats = shell.stats.get();
    ui.set_can_resume(sel.any(resumable));
    ui.set_can_stop(sel.any(stoppable));
    ui.set_can_delete(!sel.ids.is_empty());
    let loaded_active = shell
        .ctx
        .shared
        .statuses
        .lock()
        .map(|m| m.values().any(|(s, _)| s == "running"))
        .unwrap_or(false);
    ui.set_can_stop_all(stats.active > 0 || loaded_active);
    ui.set_can_delete_completed(stats.complete > 0);
    let node = shell.node.borrow().clone();
    let queue_node = node.strip_prefix("queue:").and_then(|q| q.parse::<i64>().ok());
    let (start, stop) = match queue_node.and_then(|id| shell.queues.borrow().iter().find(|q| q.id == id).cloned()) {
        Some(q) => (!q.running, q.running),
        None => (!stats.master_running, stats.master_running || stats.any_queue_running),
    };
    ui.set_can_start_queue(start);
    ui.set_can_stop_queue(stop);
}

/// Called from the refresh loop with the daemon's queue list and category rules.
pub fn on_list_reply(ui: &MainWindow, reply: &xdm_ipc::Reply, categories: Option<&[xdm_categories::CategoryRule]>) {
    let Some(shell) = SHELL.with(|s| s.borrow().clone()) else { return };
    let mut changed = false;
    let queues: Vec<QueueInfo> = reply
        .queues
        .iter()
        .map(|q| QueueInfo { id: q.id, name: q.name.clone(), running: q.running || q.started_alone })
        .collect();
    shell.stats.set(Stats {
        active: reply.queues.iter().map(|q| q.active).sum(),
        complete: reply.queues.iter().map(|q| q.complete).sum(),
        master_running: reply.queue_running == Some(true),
        any_queue_running: queues.iter().any(|q| q.running),
    });
    if shell.queues.borrow().iter().map(|q| (q.id, q.name.clone())).collect::<Vec<_>>()
        != queues.iter().map(|q| (q.id, q.name.clone())).collect::<Vec<_>>()
    {
        changed = true;
    }
    *shell.queues.borrow_mut() = queues;
    if let Some(rules) = categories {
        let names: Vec<String> = rules.iter().map(|r| r.name.clone()).collect();
        if !names.is_empty() && *shell.cats.borrow() != names {
            *shell.cats.borrow_mut() = names;
            changed = true;
        }
    }
    if changed {
        rebuild_tree(&shell);
    }
    update_toolbar(ui, &shell);
}

// ---- actions ----------------------------------------------------------------

fn say(ui: &MainWindow, text: impl Into<slint::SharedString>) {
    ui.set_message(text.into());
}

fn assign_queue(ui: &MainWindow, shell: &Shell, queue_id: i64) {
    let shared = &shell.ctx.shared;
    let target = resolve_target(ui, &shared.ids, &shared.selected, &shared.all);
    if target.count() == 0 {
        say(ui, "Select a task first");
        return;
    }
    let weak = ui.as_weak();
    let refresh = shell.ctx.refresh.clone();
    shell.ctx.ipc.execute(move |paths| {
        let command = match &target {
            Target::All(all) => bulk_command(all, xdm_ipc::BulkAction::AssignQueue { queue_id }),
            Target::Ids(ids) => Command::TaskBulk {
                action: xdm_ipc::BulkAction::AssignQueue { queue_id },
                selection: xdm_ipc::BulkSelection::Ids(ids.clone()),
                expected_total: None,
            },
        };
        let text = bulk_message(paths.request(command).map_err(|e| e.to_string()), "Assigned");
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                say(&ui, text);
            }
            refresh();
        });
    });
}

fn queue_command(ui: &MainWindow, shell: &Shell, queue_id: i64, start: bool) {
    let weak = ui.as_weak();
    let refresh = shell.ctx.refresh.clone();
    shell.ctx.ipc.execute(move |paths| {
        let command = if start { Command::QueueStartOne { queue_id } } else { Command::QueueStopOne { queue_id } };
        let text = request(paths, command);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                say(&ui, text);
            }
            refresh();
        });
    });
}

fn open_prompt(ui: &MainWindow, shell: &Rc<Shell>, caption: &str, hint: &str, initial: &str, on_ok: impl Fn(&MainWindow, String) + 'static) {
    let Ok(dialog) = PromptWindow::new() else { return };
    crate::install_tr_window(&dialog);
    dialog.set_caption(i18n::translate(caption).into());
    dialog.set_hint(hint.into());
    dialog.set_value(initial.into());
    let (weak_dialog, weak_ui) = (dialog.as_weak(), ui.as_weak());
    dialog.on_accepted(move || {
        let (Some(d), Some(ui)) = (weak_dialog.upgrade(), weak_ui.upgrade()) else { return };
        on_ok(&ui, d.get_value().to_string());
        let _ = d.hide();
    });
    let weak_dialog = dialog.as_weak();
    dialog.on_dismiss(move || {
        if let Some(d) = weak_dialog.upgrade() {
            let _ = d.hide();
        }
    });
    let _ = dialog.show();
    *shell.prompt.borrow_mut() = Some(dialog);
}

fn open_find(ui: &MainWindow, shell: &Rc<Shell>) {
    let Ok(dialog) = FindWindow::new() else { return };
    crate::install_tr_window(&dialog);
    dialog.set_query(ui.get_search_query());
    let (weak_dialog, weak_ui) = (dialog.as_weak(), ui.as_weak());
    let refresh = shell.ctx.refresh.clone();
    dialog.on_find(move || {
        let (Some(d), Some(ui)) = (weak_dialog.upgrade(), weak_ui.upgrade()) else { return };
        ui.set_search_query(d.get_query());
        refresh();
        let _ = d.hide();
    });
    let (weak_dialog, weak_ui) = (dialog.as_weak(), ui.as_weak());
    let refresh = shell.ctx.refresh.clone();
    dialog.on_show_all(move || {
        let (Some(d), Some(ui)) = (weak_dialog.upgrade(), weak_ui.upgrade()) else { return };
        ui.set_search_query("".into());
        refresh();
        let _ = d.hide();
    });
    let weak_dialog = dialog.as_weak();
    dialog.on_dismiss(move || {
        if let Some(d) = weak_dialog.upgrade() {
            let _ = d.hide();
        }
    });
    let _ = dialog.show();
    *shell.find.borrow_mut() = Some(dialog);
}

fn delete_completed(ui: &MainWindow, shell: &Rc<Shell>) {
    let weak = ui.as_weak();
    let shell2 = shell.clone();
    let filter = xdm_ipc::TaskFilter { status: Some("finished".into()), ..Default::default() };
    let f = filter.clone();
    ipc_then(
        &shell.ctx.ipc,
        move |paths| {
            paths
                .request(Command::ListPage { offset: 0, limit: 1, sort: None, filter: Some(f) })
                .map(|r| r.total)
                .map_err(|e| e.to_string())
        },
        move |result| {
            let Some(ui) = weak.upgrade() else { return };
            let total = match result {
                Ok(0) => return say(&ui, "No completed downloads"),
                Ok(total) => total,
                Err(e) => return say(&ui, e),
            };
            let Ok(dialog) = ConfirmWindow::new() else { return };
            crate::install_tr_window(&dialog);
            dialog.set_caption(format!("{} {total}? {}", i18n::translate("Delete completed downloads from the list:"), i18n::translate("Downloaded files are kept.")).into());
            let (weak_dialog, weak_ui) = (dialog.as_weak(), ui.as_weak());
            let refresh = shell2.ctx.refresh.clone();
            let ipc = shell2.ctx.ipc.clone();
            dialog.on_accepted(move || {
                let (Some(d), Some(ui)) = (weak_dialog.upgrade(), weak_ui.upgrade()) else { return };
                let _ = d.hide();
                let all = AllMatching { filter: filter.clone(), total };
                let weak = ui.as_weak();
                let refresh = refresh.clone();
                ipc.execute(move |paths| {
                    let text = bulk_message(
                        paths.request(bulk_command(&all, xdm_ipc::BulkAction::Remove)).map_err(|e| e.to_string()),
                        "Removed",
                    );
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() {
                            say(&ui, text);
                        }
                        refresh();
                    });
                });
            });
            let weak_dialog = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(d) = weak_dialog.upgrade() {
                    let _ = d.hide();
                }
            });
            let _ = dialog.show();
            *shell2.confirm.borrow_mut() = Some(dialog);
        },
    );
}

fn dispatch(ui: &MainWindow, shell: &Rc<Shell>, action: &str) {
    *shell.sub_open.borrow_mut() = None;
    match action {
        "add-url" => ui.invoke_open_add_url(),
        "add-batch" => ui.invoke_open_batch(),
        "add-media" => ui.invoke_open_add_media(),
        "grabber" => ui.invoke_open_grabber(),
        "import-export" => ui.invoke_transfer_lists(),
        "exit" => {
            let _ = slint::quit_event_loop();
        }
        "download-now" => ui.invoke_resume_task(),
        "stop" => ui.invoke_pause_task(),
        "remove" => ui.invoke_remove_task(),
        "redownload" => ui.invoke_redownload_task(),
        "restart" => ui.invoke_restart_task(),
        "open" => ui.invoke_open_task(),
        "open-folder" => ui.invoke_open_task_folder(),
        "move-rename" => ui.invoke_move_rename_task(),
        "properties" => ui.invoke_task_details(),
        "start-all" => ui.invoke_queue_start(),
        "stop-all" => ui.invoke_queue_stop(),
        "delete-completed" => delete_completed(ui, shell),
        "find" => open_find(ui, shell),
        "scheduler" => ui.invoke_queue_settings(),
        "queues" => ui.invoke_manage_queues(),
        "partials" => ui.invoke_open_partials(),
        "categories" => ui.invoke_open_categories(),
        "connection" => ui.invoke_open_connection(),
        "options" => ui.invoke_open_preferences(),
        "columns" => ui.invoke_open_columns(),
        "refresh" => (shell.ctx.refresh)(),
        "toggle-sidebar" => {
            let show = !ui.get_show_sidebar();
            ui.set_show_sidebar(show);
        }
        "toolbar-large" => ui.set_small_toolbar(false),
        "toolbar-small" => ui.set_small_toolbar(true),
        "about" => {
            if let Ok(dialog) = AboutWindow::new() {
                crate::install_tr_window(&dialog);
                dialog.set_version(xdm_ipc::ops::version_line("xdm-gui").into());
                let weak = dialog.as_weak();
                dialog.on_dismiss(move || {
                    if let Some(d) = weak.upgrade() {
                        let _ = d.hide();
                    }
                });
                let _ = dialog.show();
                *shell.about.borrow_mut() = Some(dialog);
            }
        }
        "schedule-start" => open_prompt(ui, shell, "Start the queue at (YYYY-MM-DD HH:MM):", "YYYY-MM-DD HH:MM", "", |ui, value| {
            ui.set_schedule_start(value.into());
            ui.invoke_schedule_queue_start();
        }),
        "schedule-stop" => open_prompt(ui, shell, "Stop the queue at (YYYY-MM-DD HH:MM):", "YYYY-MM-DD HH:MM", "", |ui, value| {
            ui.set_schedule_stop(value.into());
            ui.invoke_schedule_queue_stop();
        }),
        "speed:on" => {
            let initial = ui.get_rate_kib().to_string();
            open_prompt(ui, shell, "Speed limit for the selected downloads (KiB/s):", "512", &initial, |ui, value| {
                ui.set_rate_kib(value.into());
                ui.invoke_set_task_rate(true);
            });
        }
        "speed:off" => ui.invoke_set_task_rate(false),
        "speed:settings" => ui.invoke_open_bandwidth(),
        "delq" => assign_queue(ui, shell, 1),
        other => {
            if let Some(level) = other.strip_prefix("prio:") {
                ui.invoke_set_priority(level.parse().unwrap_or(1));
            } else if let Some(direction) = other.strip_prefix("move:") {
                ui.invoke_move_task(direction.into());
            } else if let Some(id) = other.strip_prefix("addq:") {
                assign_queue(ui, shell, id.parse().unwrap_or(1));
            } else if let Some(id) = other.strip_prefix("startq:") {
                queue_command(ui, shell, id.parse().unwrap_or(1), true);
            } else if let Some(id) = other.strip_prefix("stopq:") {
                queue_command(ui, shell, id.parse().unwrap_or(1), false);
            } else if let Some(key) = other.strip_prefix("sort:") {
                if let Ok(mut state) = shell.ctx.sort_state.lock() {
                    *state = (key.to_string(), true);
                    ui.set_sort_column(key.into());
                    ui.set_sort_asc(true);
                }
                (shell.ctx.refresh)();
            } else if let Some(mode) = other.strip_prefix("dbl:") {
                shell.layout.borrow_mut().double_click = mode.to_string();
            }
        }
    }
}

// ---- layout persistence -------------------------------------------------------

fn capture(ui: &MainWindow, shell: &Shell) -> Layout {
    let mut layout = shell.layout.borrow().clone();
    let window = ui.window();
    let scale = window.scale_factor().max(0.1);
    let size = window.size();
    if size.width > 0 && size.height > 0 && !window.is_minimized() {
        layout.width = (size.width as f32 / scale).round() as u32;
        layout.height = (size.height as f32 / scale).round() as u32;
        let pos = window.position();
        layout.x = Some(pos.x);
        layout.y = Some(pos.y);
    }
    layout.sidebar_width = ui.get_sidebar_width();
    layout.show_sidebar = ui.get_show_sidebar();
    layout.small_toolbar = ui.get_small_toolbar();
    layout.columns = shell
        .cols
        .iter()
        .map(|c| layout::Column { id: c.id.to_string(), width: c.width })
        .collect();
    layout.expanded = shell.expanded.borrow().iter().cloned().collect();
    layout.expanded.sort();
    layout
}

fn autosave(ui: &MainWindow, shell: &Shell) {
    let current = capture(ui, shell);
    *shell.layout.borrow_mut() = current.clone();
    if *shell.saved.borrow() != current && layout::save(&shell.ctx.dir, &current).is_ok() {
        *shell.saved.borrow_mut() = current;
    }
}

fn double_click(ui: &MainWindow, shell: &Shell) {
    let sel = selection(ui, shell);
    match shell.layout.borrow().double_click.as_str() {
        "folder" => ui.invoke_open_task_folder(),
        "properties" => ui.invoke_task_details(),
        _ if sel.single_complete() => ui.invoke_open_task(),
        _ => ui.invoke_task_details(),
    }
}

pub fn install(ui: &MainWindow, ctx: Ctx) {
    let saved = layout::load(&ctx.dir);
    let shell = Rc::new(Shell {
        ctx,
        layout: RefCell::new(saved.clone()),
        saved: RefCell::new(saved.clone()),
        tree: Rc::new(VecModel::default()),
        cols: Rc::new(VecModel::default()),
        cats: RefCell::new(BUILTIN.iter().map(|s| s.to_string()).collect()),
        queues: RefCell::new(Vec::new()),
        node: RefCell::new("all".into()),
        expanded: RefCell::new(saved.expanded.iter().cloned().collect()),
        sub_open: RefCell::new(None),
        stats: Cell::new(Stats::default()),
        find: RefCell::new(None),
        prompt: RefCell::new(None),
        confirm: RefCell::new(None),
        about: RefCell::new(None),
    });
    SHELL.with(|s| *s.borrow_mut() = Some(shell.clone()));

    // Window geometry and panes.
    let (width, height) = if saved.width >= 520 && saved.height >= 360 { (saved.width, saved.height) } else { (900, 600) };
    ui.window().set_size(LogicalSize::new(width as f32, height as f32));
    if let (Some(x), Some(y)) = (saved.x, saved.y) {
        ui.window().set_position(PhysicalPosition::new(x, y));
    }
    ui.set_sidebar_width(saved.sidebar_width);
    ui.set_show_sidebar(saved.show_sidebar);
    ui.set_small_toolbar(saved.small_toolbar);
    shell.cols.set_vec(saved.columns.iter().map(|c| ColumnSpec { id: c.id.clone().into(), width: c.width }).collect::<Vec<_>>());
    ui.set_columns(ModelRc::from(shell.cols.clone()));
    ui.set_tree(ModelRc::from(shell.tree.clone()));
    rebuild_tree(&shell);

    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_open_menu(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            *shell.sub_open.borrow_mut() = None;
            fill_menu(&ui, &shell, name.as_str());
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_menu_action(move |action| {
            if let Some(ui) = weak.upgrade() {
                dispatch(&ui, &shell, action.as_str());
            }
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_submenu_requested(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            if shell.sub_open.borrow().as_deref() == Some(name.as_str()) {
                return;
            }
            *shell.sub_open.borrow_mut() = Some(name.to_string());
            fill_sub(&ui, &shell, name.as_str());
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_task_right_clicked(move |index, _, _| {
            let Some(ui) = weak.upgrade() else { return };
            *shell.sub_open.borrow_mut() = None;
            let shared = &shell.ctx.shared;
            let id = usize::try_from(index).ok().and_then(|i| shared.ids.lock().ok().and_then(|ids| id_at(&ids, i)));
            if let (Some(id), Ok(mut set)) = (id, shared.selected.lock()) {
                if !set.contains(&(id as usize)) {
                    set.clear();
                    set.insert(id as usize);
                    sync_selection(&ui, &set);
                }
            }
            fill_context(&ui, &shell);
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_task_double_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                double_click(&ui, &shell);
            }
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_tree_clicked(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if id.as_str() == "grabber" {
                ui.invoke_open_grabber();
                return;
            }
            apply_node(&ui, &shell, id.as_str());
        });
    }
    {
        let shell = shell.clone();
        ui.on_tree_toggled(move |id| {
            {
                let mut set = shell.expanded.borrow_mut();
                if !set.remove(id.as_str()) {
                    set.insert(id.to_string());
                }
            }
            rebuild_tree(&shell);
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_columns_changed(move || {
            if let Some(ui) = weak.upgrade() {
                autosave(&ui, &shell);
            }
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_column_dragged(move |from, dx| {
            let Some(ui) = weak.upgrade() else { return };
            let from = from as usize;
            let columns: Vec<layout::Column> =
                shell.cols.iter().map(|c| layout::Column { id: c.id.to_string(), width: c.width }).collect();
            if from >= columns.len() {
                return;
            }
            let to = layout::drop_index(&columns, |id| visible_flag(&ui, id), from, dx);
            if to != from {
                let mut columns = columns;
                let moved = columns.remove(from);
                columns.insert(to, moved);
                shell.cols.set_vec(columns.iter().map(|c| ColumnSpec { id: c.id.clone().into(), width: c.width }).collect::<Vec<_>>());
                autosave(&ui, &shell);
            }
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_delete_completed(move || {
            if let Some(ui) = weak.upgrade() {
                delete_completed(&ui, &shell);
            }
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_open_find(move || {
            if let Some(ui) = weak.upgrade() {
                open_find(&ui, &shell);
            }
        });
    }
    {
        let (weak, shell) = (ui.as_weak(), shell.clone());
        ui.on_toolbar_queue(move |start| {
            let Some(ui) = weak.upgrade() else { return };
            let node = shell.node.borrow().clone();
            match node.strip_prefix("queue:").and_then(|q| q.parse::<i64>().ok()) {
                Some(id) => queue_command(&ui, &shell, id, start),
                None if start => ui.invoke_queue_start(),
                None => ui.invoke_queue_stop(),
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_hide_sidebar(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_sidebar(false);
            }
        });
    }

    // Toolbar enable states follow the selection; layout autosaves when it changes.
    let timer = Timer::default();
    let (weak, tick_shell) = (ui.as_weak(), shell.clone());
    let ticks = Cell::new(0u32);
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        let Some(ui) = weak.upgrade() else { return };
        update_toolbar(&ui, &tick_shell);
        let n = ticks.get().wrapping_add(1);
        ticks.set(n);
        if n % 4 == 0 {
            autosave(&ui, &tick_shell);
        }
    });
    std::mem::forget(timer);
}
