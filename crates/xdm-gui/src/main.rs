mod clipboard;
mod dialog_helpers;
mod i18n;
mod maintenance;
mod paged;
mod scheduler;
mod settings;
mod sound;
mod task_table;

use chrono::{Local, NaiveDateTime, TimeZone};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use paged::{FetchPage, PAGE, PagedModel, Query};
use xdm_ipc::{Command, CompletionAction, MoveTarget, Paths, TaskSummary};

slint::slint! {
    import { Button, CheckBox, LineEdit, ListView, ScrollView, TextEdit, VerticalBox, HorizontalBox, Palette } from "std-widgets.slint";
    import "assets/DejaVuSans.ttf";

    export global Tr {
        in-out property <string> lang: "en";
        pure callback t(string, string) -> string;
    }

    export struct TaskRow {
        id: int,
        filename: string,
        size: string,
        status: string,
        time_left: string,
        transfer_rate: string,
        last_try_date: string,
        description: string,
        category: string,
        queue: string,
        progress: float,
        selected: bool,
    }

    component HeaderCell inherits Rectangle {
        in property <string> text;
        callback clicked;
        height: 28px;
        border-radius: 3px;
        background: ta.has-hover ? (Palette.color-scheme == ColorScheme.dark ? #3a4250 : #d3dcea) : (Palette.color-scheme == ColorScheme.dark ? #2c313a : #e3e9f2);
        Text {
            x: 6px;
            width: parent.width - 12px;
            text: root.text;
            overflow: elide;
            vertical-alignment: center;
            font-weight: 700;
            color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
        }
        ta := TouchArea { clicked => { root.clicked(); } }
    }

    export component MainWindow inherits Window {
        title: "XDM — Downloads";
        width: 900px;
        height: 780px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        public function apply-theme(theme: string) {
            if (theme == "dark") {
                Palette.color-scheme = ColorScheme.dark;
            } else if (theme == "light") {
                Palette.color-scheme = ColorScheme.light;
            } else {
                Palette.color-scheme = ColorScheme.unknown;
            }
        }
        in property <[TaskRow]> tasks;
        in-out property <string> url;
        in-out property <string> destination;
        in-out property <string> message: "Connecting to daemon…";
        in-out property <string> queue-state: "Queue status unknown";
        in-out property <string> schedule-start;
        in-out property <string> schedule-stop;
        in-out property <int> selected: -1;
        in-out property <string> category-filter: "All";
        in-out property <string> search-query;
        in-out property <string> rate-kib: "512";
        in-out property <bool> tools-expanded: false;
        in-out property <string> queue-filter-id: "0";
        in-out property <string> queue-caption: "All queues";
        in-out property <bool> has-pending-completion: false;
        in-out property <string> pending-action-text: "";
        in-out property <int> pending-queue-id: 0;
        in-out property <int> pending-seconds-remaining: 0;
        in-out property <string> pause-button-text: "Pause";
        in-out property <bool> col-filename: true;
        in-out property <bool> col-size: true;
        in-out property <bool> col-status: true;
        in-out property <bool> col-time-left: true;
        in-out property <bool> col-transfer-rate: true;
        in-out property <bool> col-last-try-date: true;
        in-out property <bool> col-description: true;
        in-out property <bool> col-category: true;
        in-out property <bool> col-queue: true;
        in-out property <string> sort-column: "id";
        in-out property <bool> sort-asc: true;
        in-out property <length> context-menu-x: 100px;
        in-out property <length> context-menu-y: 200px;
        callback cancel-completion(int);
        callback add-task();
        callback pause-task();
        callback resume-task();
        callback set-priority(int);
        callback set-task-rate(bool);
        callback task-details();
        callback redownload-task();
        callback remove-task();
        callback transfer-lists();
        callback manage-queues();
        callback refresh();
        callback queue-start();
        callback queue-stop();
        callback schedule-queue-start();
        callback schedule-queue-stop();
        callback move-task(string);
        callback queue-start-one();
        callback queue-stop-one();
        callback queue-settings();
        callback open-batch();
        callback open-grabber();
        callback open-categories();
        callback open-connection();
        callback open-preferences();
        callback open-add-url();
        callback open-add-media();
        callback task-selected(int);
        callback show-clipboard-prompt(string);
        callback sort-by-column(string);
        callback open-columns();
        callback open-task();
        callback open-task-folder();
        callback move-rename-task();
        callback restart-task();
        callback open-partials();
        callback open-bandwidth();
        callback task-clicked(int, bool, bool);
        callback task-right-clicked(int, length, length);
        callback key-delete();
        callback key-enter();
        callback key-up();
        callback key-down();
        callback key-select-all();

        forward-focus: fs;
        fs := FocusScope {
            key-pressed(event) => {
                if (event.text == Key.Delete) {
                    root.key-delete();
                    return accept;
                }
                if (event.text == Key.Return) {
                    root.key-enter();
                    return accept;
                }
                if (event.text == Key.UpArrow) {
                    root.key-up();
                    return accept;
                }
                if (event.text == Key.DownArrow) {
                    root.key-down();
                    return accept;
                }
                if (event.modifiers.control && (event.text == "a" || event.text == "A")) {
                    root.key-select-all();
                    return accept;
                }
                return reject;
            }
            VerticalBox {
                padding: 14px;
                spacing: 10px;
                if root.has-pending-completion: Rectangle {
                    background: #ffcdd2;
                    border-width: 2px;
                    border-color: #b71c1c;
                    border-radius: 4px;
                    height: 44px;
                    HorizontalBox {
                        padding-left: 12px;
                        padding-right: 12px;
                        spacing: 16px;
                        alignment: space-between;
                        Text {
                            text: "⚠ WARNING: " + root.pending-action-text;
                            font-size: 13px;
                            font-weight: 700;
                            color: #b71c1c;
                            vertical-alignment: center;
                        }
                        Button {
                            text: "Cancel";
                            clicked => { root.cancel-completion(root.pending-queue-id); }
                        }
                    }
                }
                HorizontalBox {
                    padding: 0px;
                    Text { text: Tr.t(@tr("XDM"), Tr.lang); font-size: 26px; font-weight: 700; color: #143f73; }
                    Button { text: Tr.t(@tr("All queues"), Tr.lang); clicked => { root.queue-filter-id = "0"; root.refresh(); } }
                    Button { text: Tr.t(@tr("Queues…"), Tr.lang); clicked => { root.manage-queues(); } }
                }
                HorizontalBox {
                    spacing: 8px;
                    Button { text: Tr.t(@tr("Add URL"), Tr.lang); clicked => { root.open-add-url(); } }
                    Button { text: Tr.t(@tr("Batch…"), Tr.lang); clicked => { root.open-batch(); } }
                    Button { text: Tr.t(@tr("Grabber…"), Tr.lang); clicked => { root.open-grabber(); } }
                    Button { text: Tr.t(@tr("Categories…"), Tr.lang); clicked => { root.open-categories(); } }
                    Button { text: Tr.t(@tr("Connection…"), Tr.lang); clicked => { root.open-connection(); } }
                    Button { text: root.pause-button-text == "Pause" ? Tr.t(@tr("Pause"), Tr.lang) : Tr.t(@tr("Stop and save"), Tr.lang); clicked => { root.pause-task(); } }
                    Button { text: Tr.t(@tr("Resume"), Tr.lang); clicked => { root.resume-task(); } }
                    Button { text: Tr.t(@tr("Refresh"), Tr.lang); clicked => { root.refresh(); } }
                }
                HorizontalBox {
                    spacing: 8px;
                    Button { text: Tr.t(@tr("Details"), Tr.lang); clicked => { root.task-details(); } }
                    Button { text: Tr.t(@tr("Redownload…"), Tr.lang); clicked => { root.redownload-task(); } }
                    Button { text: Tr.t(@tr("Remove from list…"), Tr.lang); clicked => { root.remove-task(); } }
                    Button { text: Tr.t(@tr("Open"), Tr.lang); clicked => { root.open-task(); } }
                    Button { text: Tr.t(@tr("Open folder"), Tr.lang); clicked => { root.open-task-folder(); } }
                    Button { text: Tr.t(@tr("Move / Rename…"), Tr.lang); clicked => { root.move-rename-task(); } }
                }
                HorizontalBox {
                    spacing: 8px;
                    Button { text: Tr.t(@tr("Start all"), Tr.lang); clicked => { root.queue-start(); } }
                    Button { text: Tr.t(@tr("Stop all"), Tr.lang); clicked => { root.queue-stop(); } }
                    Button { text: Tr.t(@tr("Import / Export…"), Tr.lang); clicked => { root.transfer-lists(); } }
                    Button { text: Tr.t(@tr("Add media…"), Tr.lang); clicked => { root.open-add-media(); } }
                    Text { text: root.queue-state; color: #38536f; vertical-alignment: center; }
                    Button { text: root.tools-expanded ? Tr.t(@tr("Hide controls"), Tr.lang) : Tr.t(@tr("More controls"), Tr.lang); clicked => { root.tools-expanded = !root.tools-expanded; } }
                }
                if root.tools-expanded: Rectangle {
                    height: 220px;
                    ScrollView {
                        VerticalBox {
                            padding: 4px;
                            spacing: 6px;
                            HorizontalBox {
                                Text { text: "Tools:"; vertical-alignment: center; }
                                Button { text: Tr.t(@tr("Bandwidth…"), Tr.lang); clicked => { root.open-bandwidth(); } }
                                Button { text: Tr.t(@tr("Clean up partial files…"), Tr.lang); clicked => { root.open-partials(); } }
                                Button { text: Tr.t(@tr("Restart"), Tr.lang); clicked => { root.restart-task(); } }
                            }
                            HorizontalBox {
                                Text { text: "URL"; vertical-alignment: center; }
                                LineEdit { text <=> root.url; placeholder-text: "https://example.com/file.zip or ftp://..."; }
                            }
                            HorizontalBox {
                                Text { text: "Save as"; vertical-alignment: center; }
                                LineEdit { text <=> root.destination; placeholder-text: "/home/user/Downloads/file.zip"; }
                                Button { text: Tr.t(@tr("Download"), Tr.lang); clicked => { root.add-task(); } }
                            }
                            HorizontalBox {
                                Text { text: "Priority:"; vertical-alignment: center; }
                                Button { text: Tr.t(@tr("Immediate"), Tr.lang); clicked => { root.set-priority(2); } }
                                Button { text: Tr.t(@tr("Normal"), Tr.lang); clicked => { root.set-priority(1); } }
                                Button { text: Tr.t(@tr("Low"), Tr.lang); clicked => { root.set-priority(0); } }
                            }
                            HorizontalBox {
                                LineEdit { text <=> root.schedule-start; placeholder-text: "YYYY-MM-DD HH:MM"; }
                                Button { text: Tr.t(@tr("Schedule start"), Tr.lang); clicked => { root.schedule-queue-start(); } }
                                LineEdit { text <=> root.schedule-stop; placeholder-text: "Stop at YYYY-MM-DD HH:MM"; }
                                Button { text: Tr.t(@tr("Schedule stop"), Tr.lang); clicked => { root.schedule-queue-stop(); } }
                            }
                            HorizontalBox {
                                Text { text: "Selected limit (KiB/s)"; vertical-alignment: center; }
                                LineEdit { text <=> root.rate-kib; }
                                Button { text: Tr.t(@tr("Apply limit"), Tr.lang); clicked => { root.set-task-rate(true); } }
                                Button { text: Tr.t(@tr("Unlimited"), Tr.lang); clicked => { root.set-task-rate(false); } }
                            }
                        }
                    }
                }
                HorizontalBox {
                    spacing: 8px;
                    LineEdit { text <=> root.search-query; placeholder-text: "Find URL or file name (status:paused filters by status)"; }
                    Button { text: Tr.t(@tr("Find"), Tr.lang); clicked => { root.refresh(); } }
                    Button { text: Tr.t(@tr("Clear"), Tr.lang); clicked => { root.search-query = ""; root.refresh(); } }
                    Button { text: Tr.t(@tr("Columns…"), Tr.lang); clicked => { root.open-columns(); } }
                    Button { text: Tr.t(@tr("Preferences…"), Tr.lang); clicked => { root.open-preferences(); } }
                }
                Rectangle { height: 1px; background: #c8d0db; }
                HorizontalBox {
                    spacing: 4px;
                    Button { text: Tr.t(@tr("All"), Tr.lang); clicked => { root.category-filter = "All"; root.refresh(); } }
                    Button { text: Tr.t(@tr("Video"), Tr.lang); clicked => { root.category-filter = "Video"; root.refresh(); } }
                    Button { text: Tr.t(@tr("Music"), Tr.lang); clicked => { root.category-filter = "Music"; root.refresh(); } }
                    Button { text: Tr.t(@tr("Documents"), Tr.lang); clicked => { root.category-filter = "Documents"; root.refresh(); } }
                    Button { text: Tr.t(@tr("Compressed"), Tr.lang); clicked => { root.category-filter = "Compressed"; root.refresh(); } }
                    Button { text: Tr.t(@tr("Programs"), Tr.lang); clicked => { root.category-filter = "Programs"; root.refresh(); } }
                    Button { text: Tr.t(@tr("Other"), Tr.lang); clicked => { root.category-filter = "Other"; root.refresh(); } }
                }
                HorizontalBox {
                    padding: 0px;
                    spacing: 8px;
                    Text { text: Tr.t(@tr("Downloads"), Tr.lang) + "  ·  " + root.queue-caption; font-size: 14px; color: #38536f; vertical-alignment: center; }
                    Button { text: Tr.t(@tr("Start queue"), Tr.lang); enabled: root.queue-filter-id != "0"; clicked => { root.queue-start-one(); } }
                    Button { text: Tr.t(@tr("Stop queue"), Tr.lang); enabled: root.queue-filter-id != "0"; clicked => { root.queue-stop-one(); } }
                    Button { text: Tr.t(@tr("Scheduler…"), Tr.lang); clicked => { root.queue-settings(); } }
                    Button { text: Tr.t(@tr("Top"), Tr.lang); clicked => { root.move-task("top"); } }
                    Button { text: Tr.t(@tr("Up"), Tr.lang); clicked => { root.move-task("up"); } }
                    Button { text: Tr.t(@tr("Down"), Tr.lang); clicked => { root.move-task("down"); } }
                    Button { text: Tr.t(@tr("Bottom"), Tr.lang); clicked => { root.move-task("bottom"); } }
                }
                HorizontalLayout {
                    height: 28px;
                    padding-left: 6px;
                    padding-right: 20px;
                    spacing: 6px;
                    if root.col-filename: HeaderCell {
                        text: Tr.t(@tr("File name"), Tr.lang) + (root.sort-column == "filename" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        horizontal-stretch: 3;
                        clicked => { root.sort-by-column("filename"); }
                    }
                    if root.col-size: HeaderCell {
                        text: Tr.t(@tr("Size"), Tr.lang) + (root.sort-column == "size" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        min-width: 70px;
                        max-width: 70px;
                        clicked => { root.sort-by-column("size"); }
                    }
                    if root.col-status: HeaderCell {
                        text: Tr.t(@tr("Status"), Tr.lang) + (root.sort-column == "status" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        min-width: 120px;
                        max-width: 120px;
                        clicked => { root.sort-by-column("status"); }
                    }
                    if root.col-time-left: HeaderCell {
                        text: Tr.t(@tr("Time left"), Tr.lang) + (root.sort-column == "time_left" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        min-width: 70px;
                        max-width: 70px;
                        clicked => { root.sort-by-column("time_left"); }
                    }
                    if root.col-transfer-rate: HeaderCell {
                        text: Tr.t(@tr("Transfer rate"), Tr.lang) + (root.sort-column == "transfer_rate" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        min-width: 90px;
                        max-width: 90px;
                        clicked => { root.sort-by-column("transfer_rate"); }
                    }
                    if root.col-last-try-date: HeaderCell {
                        text: Tr.t(@tr("Last try date"), Tr.lang) + (root.sort-column == "last_try_date" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        min-width: 160px;
                        max-width: 160px;
                        clicked => { root.sort-by-column("last_try_date"); }
                    }
                    if root.col-description: HeaderCell {
                        text: Tr.t(@tr("Description"), Tr.lang) + (root.sort-column == "description" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        horizontal-stretch: 2;
                        clicked => { root.sort-by-column("description"); }
                    }
                    if root.col-category: HeaderCell {
                        text: Tr.t(@tr("Category"), Tr.lang) + (root.sort-column == "category" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        min-width: 70px;
                        max-width: 70px;
                        clicked => { root.sort-by-column("category"); }
                    }
                    if root.col-queue: HeaderCell {
                        text: Tr.t(@tr("Queue"), Tr.lang) + (root.sort-column == "queue" ? (root.sort-asc ? " ▲" : " ▼") : "");
                        min-width: 60px;
                        max-width: 60px;
                        clicked => { root.sort-by-column("queue"); }
                    }
                    
                }
                ListView {
                    for task[index] in root.tasks: Rectangle {
                        height: 40px;
                        background: task.selected ? (Palette.color-scheme == ColorScheme.dark ? #2c4260 : #d8eaff)
                            : (root.selected == index ? (Palette.color-scheme == ColorScheme.dark ? #28374a : #eef4fc)
                            : (Math.mod(index, 2) == 0 ? (Palette.color-scheme == ColorScheme.dark ? #1e2227 : #ffffff)
                                                       : (Palette.color-scheme == ColorScheme.dark ? #21252b : #f7f9fc)));
                        HorizontalLayout {
                            padding-left: 6px;
                            padding-right: 6px;
                            spacing: 6px;
                            if root.col-filename: Text {
                                horizontal-stretch: 3;
                                preferred-width: 1px;
                                text: task.filename;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-size: Text {
                                min-width: 70px;
                                max-width: 70px;
                                text: task.size;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-status: Text {
                                min-width: 120px;
                                max-width: 120px;
                                text: task.status;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-time-left: Text {
                                min-width: 70px;
                                max-width: 70px;
                                text: task.time-left;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-transfer-rate: Text {
                                min-width: 90px;
                                max-width: 90px;
                                text: task.transfer-rate;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-last-try-date: Text {
                                min-width: 160px;
                                max-width: 160px;
                                text: task.last-try-date;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-description: Text {
                                horizontal-stretch: 2;
                                preferred-width: 1px;
                                text: task.description;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-category: Text {
                                min-width: 70px;
                                max-width: 70px;
                                text: task.category;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                            if root.col-queue: Text {
                                min-width: 60px;
                                max-width: 60px;
                                text: task.queue;
                                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                                overflow: elide;
                                vertical-alignment: center;
                            }
                        }
                        Rectangle {
                            y: parent.height - 3px;
                            width: parent.width * task.progress / 100;
                            height: 3px;
                            background: #2874c7;
                        }
                        TouchArea {
                            pointer-event(event) => {
                                if (event.button == PointerEventButton.left && event.kind == PointerEventKind.down) {
                                    root.selected = index;
                                    fs.focus();
                                    root.task-clicked(index, event.modifiers.control, event.modifiers.shift);
                                    root.task-selected(index);
                                }
                                if (event.button == PointerEventButton.right && event.kind == PointerEventKind.down) {
                                    root.selected = index;
                                    root.context-menu-x = min(root.width - 180px, max(10px, self.absolute-position.x + self.mouse-x - root.absolute-position.x));
                                    root.context-menu-y = min(root.height - 260px, max(10px, self.absolute-position.y + self.mouse-y - root.absolute-position.y));
                                    root.task-right-clicked(index, self.mouse-x, self.mouse-y);
                                    context-popup.show();
                                }
                            }
                        }
                    }
                }
                Text { text: root.message; color: #52677c; }
            }
        }

        context-popup := PopupWindow {
            x: root.context-menu-x;
            y: root.context-menu-y;
            width: 170px;
            height: 240px;
            close-policy: close-on-click-outside;
            Rectangle {
                background: Palette.color-scheme == ColorScheme.dark ? #282c34 : #ffffff;
                border-width: 1px;
                border-color: Palette.color-scheme == ColorScheme.dark ? #3e4451 : #c8d0db;
                border-radius: 4px;
                drop-shadow-blur: 8px;
                drop-shadow-color: #00000033;
                VerticalBox {
                    padding: 4px;
                    spacing: 2px;
                    Button { text: Tr.t(@tr("Open"), Tr.lang); clicked => { context-popup.close(); root.open-task(); } }
                    Button { text: Tr.t(@tr("Open folder"), Tr.lang); clicked => { context-popup.close(); root.open-task-folder(); } }
                    Button { text: Tr.t(@tr("Move / Rename…"), Tr.lang); clicked => { context-popup.close(); root.move-rename-task(); } }
                    Button { text: Tr.t(@tr("Redownload…"), Tr.lang); clicked => { context-popup.close(); root.redownload-task(); } }
                    Button { text: Tr.t(@tr("Restart"), Tr.lang); clicked => { context-popup.close(); root.restart-task(); } }
                    Button { text: Tr.t(@tr("Delete"), Tr.lang); clicked => { context-popup.close(); root.remove-task(); } }
                    Button { text: Tr.t(@tr("Properties"), Tr.lang); clicked => { context-popup.close(); root.task-details(); } }
                }
            }
        }
    }

    export component DetailsWindow inherits Window {
        title: "XDM — Download details";
        width: 640px;
        height: 480px;
        in property <string> details;
        callback dismiss();
        VerticalBox {
            padding: 14px;
            ScrollView { Text { text: root.details; wrap: word-wrap; } }
            Button { text: "Close"; clicked => { root.dismiss(); } }
        }
    }

    export component RedownloadWindow inherits Window {
        title: "XDM — Redownload";
        width: 640px;
        height: 260px;
        in-out property <string> output;
        in-out property <string> message: "Choose a fresh destination. The earlier file will be kept.";
        callback submit();
        callback dismiss();
        VerticalBox {
            padding: 14px;
            Text { text: "Download again from the beginning"; font-size: 18px; }
            Text { text: "Save as"; }
            LineEdit { text <=> root.output; }
            Text { text: root.message; wrap: word-wrap; }
            HorizontalBox {
                Button { text: "Add to queue"; clicked => { root.submit(); } }
                Button { text: "Cancel"; clicked => { root.dismiss(); } }
            }
        }
    }

    export component RemoveWindow inherits Window {
        title: "XDM — Remove task";
        width: 480px;
        height: 260px;
        in-out property <string> message;
        in property <bool> partial-visible;
        in-out property <bool> delete-partial;
        callback confirm();
        callback dismiss();
        VerticalBox {
            padding: 14px;
            Text { text: root.message; wrap: word-wrap; }
            Text {
                text: root.delete-partial ? "The partial file and checkpoint of unfinished tasks will be deleted." : "Downloaded files and checkpoints will be kept.";
                wrap: word-wrap;
            }
            Rectangle {
                height: 36px;
                if root.partial-visible: CheckBox {
                    text: "Also delete partial file";
                    checked <=> root.delete-partial;
                }
            }
            HorizontalBox {
                Button { text: "Remove from list"; clicked => { root.confirm(); } }
                Button { text: "Cancel"; clicked => { root.dismiss(); } }
            }
        }
    }

    export component MoveRenameWindow inherits Window {
        title: Tr.t(@tr("XDM — Move / Rename completed download"), Tr.lang);
        width: 600px;
        height: 220px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> current-path;
        in-out property <string> target-path;
        in-out property <string> message: Tr.t(@tr("Select a new destination path for the completed file."), Tr.lang);
        callback submit();
        callback dismiss();
        VerticalBox {
            padding: 14px;
            spacing: 8px;
            Text { text: Tr.t(@tr("Current path: "), Tr.lang) + root.current-path; overflow: elide; color: #52677c; }
            Text { text: Tr.t(@tr("New destination path:"), Tr.lang); }
            LineEdit { text <=> root.target-path; }
            Text { text: root.message; wrap: word-wrap; color: #d32f2f; }
            HorizontalBox {
                Button { text: Tr.t(@tr("Move / Rename"), Tr.lang); clicked => { root.submit(); } }
                Button { text: Tr.t(@tr("Cancel"), Tr.lang); clicked => { root.dismiss(); } }
            }
        }
    }

    export component ColumnsWindow inherits Window {
        title: Tr.t(@tr("XDM — Customize columns"), Tr.lang);
        width: 380px;
        height: 480px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <bool> col-filename: true;
        in-out property <bool> col-size: true;
        in-out property <bool> col-status: true;
        in-out property <bool> col-time-left: true;
        in-out property <bool> col-transfer-rate: true;
        in-out property <bool> col-last-try-date: true;
        in-out property <bool> col-description: true;
        in-out property <bool> col-category: true;
        in-out property <bool> col-queue: true;
        callback apply-columns();
        callback dismiss();
        VerticalBox {
            padding: 16px;
            spacing: 8px;
            Text { text: Tr.t(@tr("Select columns to display:"), Tr.lang); font-size: 16px; font-weight: 700; }
            CheckBox { text: Tr.t(@tr("File name"), Tr.lang); checked <=> root.col-filename; }
            CheckBox { text: Tr.t(@tr("Size"), Tr.lang); checked <=> root.col-size; }
            CheckBox { text: Tr.t(@tr("Status"), Tr.lang); checked <=> root.col-status; }
            CheckBox { text: Tr.t(@tr("Time left"), Tr.lang); checked <=> root.col-time-left; }
            CheckBox { text: Tr.t(@tr("Transfer rate"), Tr.lang); checked <=> root.col-transfer-rate; }
            CheckBox { text: Tr.t(@tr("Last try date"), Tr.lang); checked <=> root.col-last-try-date; }
            CheckBox { text: Tr.t(@tr("Description / URL"), Tr.lang); checked <=> root.col-description; }
            CheckBox { text: Tr.t(@tr("Category"), Tr.lang); checked <=> root.col-category; }
            CheckBox { text: Tr.t(@tr("Queue"), Tr.lang); checked <=> root.col-queue; }
            HorizontalBox {
                Button { text: Tr.t(@tr("Save"), Tr.lang); clicked => { root.apply-columns(); } }
                Button { text: Tr.t(@tr("Close"), Tr.lang); clicked => { root.dismiss(); } }
            }
        }
    }

    export component ListsWindow inherits Window {
        title: "XDM — Import and export";
        width: 700px;
        height: 560px;
        in-out property <string> input-file;
        in-out property <string> directory;
        in-out property <string> export-file;
        in-out property <string> preview;
        in-out property <string> message: "Preview a URL list or JSON task list before importing.";
        in-out property <bool> preview-ready: false;
        callback preview-list();
        callback import-list(bool);
        callback export-list(bool);
        callback dismiss();
        VerticalBox {
            padding: 14px;
            spacing: 8px;
            Text { text: "Import downloads"; font-size: 18px; }
            LineEdit { text <=> root.input-file; placeholder-text: "URL list or JSON file path"; }
            LineEdit { text <=> root.directory; placeholder-text: "Absolute destination directory"; }
            HorizontalBox {
                Button { text: "Preview"; clicked => { root.preview-list(); } }
                Button { text: "Import paused"; enabled: root.preview-ready; clicked => { root.import-list(false); } }
                Button { text: "Add to queue"; enabled: root.preview-ready; clicked => { root.import-list(true); } }
            }
            ScrollView { height: 160px; Text { text: root.preview; wrap: word-wrap; } }
            Text { text: "Export downloads"; font-size: 18px; }
            LineEdit { text <=> root.export-file; placeholder-text: "New export file path"; }
            HorizontalBox {
                Button { text: "Export JSON"; clicked => { root.export-list(true); } }
                Button { text: "Export URLs"; clicked => { root.export-list(false); } }
                Button { text: "Close"; clicked => { root.dismiss(); } }
            }
            Text { text: root.message; wrap: word-wrap; }
        }
    }

    export component QueuesWindow inherits Window {
        title: "XDM — Queues";
        width: 780px;
        height: 652px;
        in property <[string]> queues;
        in-out property <int> selected: -1;
        in-out property <string> sync-interval: "3600";
        in-out property <string> name;
        in-out property <string> limit: "2";
        in-out property <string> message;
        in property <bool> can-assign;
        callback select-queue(int);
        callback action(int);
        callback refresh-queues();
        callback open-settings();
        callback dismiss();
        VerticalBox {
            padding: 14px;
            spacing: 8px;
            Text { text: "Download queues"; font-size: 18px; }
            ListView {
                for queue[index] in root.queues: Rectangle {
                    height: 38px;
                    background: root.selected == index ? #d8eaff : #ffffff;
                    Text { x: 8px; y: 8px; text: queue; overflow: elide; width: parent.width - 16px; }
                    TouchArea { clicked => { root.selected = index; root.select-queue(index); } }
                }
            }
            HorizontalBox {
                LineEdit { text <=> root.name; placeholder-text: "Queue name"; }
                Text { text: "Job limit"; vertical-alignment: center; }
                LineEdit { text <=> root.limit; preferred-width: 80px; }
            }
            HorizontalBox {
                Button { text: "Create"; clicked => { root.action(0); } }
                Button { text: "Rename"; clicked => { root.action(1); } }
                Button { text: "Apply limit"; clicked => { root.action(2); } }
                Button { text: "Remove empty"; clicked => { root.action(5); } }
                Button { text: "Refresh"; clicked => { root.refresh-queues(); } }
            }
            HorizontalBox {
                Button { text: "Start selected"; clicked => { root.action(3); } }
                Button { text: "Stop selected"; clicked => { root.action(4); } }
                Button { text: "Show selected queue"; clicked => { root.action(7); } }
            }
            Button { text: "Move the selected download to this queue"; enabled: root.can-assign; clicked => { root.action(6); } }
            HorizontalBox {
                Button { text: "Start now (solo)"; clicked => { root.action(8); } }
                Button { text: "Stop now (solo)"; clicked => { root.action(9); } }
                Button { text: "Scheduler & Settings…"; clicked => { root.open-settings(); } }
            }
            HorizontalBox {
                Button { text: "Create sync queue"; clicked => { root.action(10); } }
                Text { text: "Check every (s)"; vertical-alignment: center; }
                LineEdit { text <=> root.sync-interval; preferred-width: 90px; }
                Button { text: "Set interval"; clicked => { root.action(11); } }
                Button { text: "Manual only"; clicked => { root.action(12); } }
                Button { text: "Check now"; clicked => { root.action(13); } }
            }
            Text { text: root.message; wrap: word-wrap; }
            Button { text: "Close"; clicked => { root.dismiss(); } }
        }
    }

    export component QueueSettingsWindow inherits Window {
        title: "XDM — Queue settings";
        width: 820px;
        height: 650px;
        background: #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> queue-title: "Queue settings";
        in-out property <string> status-text: "Status: stopped";
        in-out property <string> retry-count: "3";
        in-out property <string> retry-delay: "30";
        in-out property <bool> retry-infinite: false;
        in-out property <string> schedule-kind: "none";
        in-out property <string> once-start: "";
        in-out property <string> once-stop: "";
        in-out property <string> daily-start: "09:00";
        in-out property <string> daily-stop: "17:00";
        in-out property <bool> day-mon: true;
        in-out property <bool> day-tue: true;
        in-out property <bool> day-wed: true;
        in-out property <bool> day-thu: true;
        in-out property <bool> day-fri: true;
        in-out property <bool> day-sat: false;
        in-out property <bool> day-sun: false;
        in-out property <string> completion-action: "none";
        in-out property <string> countdown-seconds: "60";
        in-out property <bool> confirm-destructive: false;
        in-out property <string> program-path: "";
        in-out property <string> program-args: "";
        in-out property <bool> is-destructive: root.completion-action == "sleep" || root.completion-action == "hibernate" || root.completion-action == "shutdown" || root.completion-action == "restart";
        in-out property <string> confirm-label: "I understand this will " + root.completion-action + " the computer";
        in property <[string]> events;
        in-out property <string> message: "Ready";
        callback start-solo();
        callback stop-solo();
        callback save-retry();
        callback save-schedule();
        callback select-completion(string);
        callback save-completion();
        callback save-all();
        callback refresh-events();
        callback select-kind(string);
        callback select-weekdays(bool);
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 8px;
            Text { text: root.queue-title; font-size: 20px; font-weight: 700; color: #143f73; }
            Text { text: root.status-text; font-size: 13px; color: #38536f; }
            ScrollView {
                VerticalBox {
                    padding: 0px;
                    spacing: 8px;
                    HorizontalBox {
                        spacing: 8px;
                        Button { text: "Start queue now"; clicked => { root.start-solo(); } }
                        Button { text: "Stop queue"; clicked => { root.stop-solo(); } }
                        Button { text: "Refresh events"; clicked => { root.refresh-events(); } }
                    }
                    Rectangle { height: 1px; background: #c8d0db; }
                    Text { text: "Retry Policy"; font-size: 15px; font-weight: 700; color: #143f73; }
                    HorizontalBox {
                        spacing: 8px;
                        Text { text: "Retries:"; vertical-alignment: center; }
                        LineEdit { text <=> root.retry-count; enabled: !root.retry-infinite; preferred-width: 90px; }
                        Button {
                            text: root.retry-infinite ? "Infinite [Active]" : "Set infinite";
                            clicked => {
                                root.retry-infinite = !root.retry-infinite;
                                if (root.retry-infinite) {
                                    root.retry-count = "infinite";
                                } else {
                                    root.retry-count = "3";
                                }
                            }
                        }
                        Text { text: "Delay (s):"; vertical-alignment: center; }
                        LineEdit { text <=> root.retry-delay; preferred-width: 80px; }
                        Button { text: "Apply retry"; clicked => { root.save-retry(); } }
                    }
                    Rectangle { height: 1px; background: #c8d0db; }
                    Text { text: "Schedule (IDM Scheduler)"; font-size: 15px; font-weight: 700; color: #143f73; }
                    HorizontalBox {
                        spacing: 8px;
                        Text { text: "Mode:"; vertical-alignment: center; }
                        Button { text: root.schedule-kind == "none" ? "[ None ]" : "None"; clicked => { root.select-kind("none"); } }
                        Button { text: root.schedule-kind == "once" ? "[ One-time ]" : "One-time"; clicked => { root.select-kind("once"); } }
                        Button { text: root.schedule-kind == "daily" ? "[ Daily ]" : "Daily"; clicked => { root.select-kind("daily"); } }
                        Button { text: "Apply schedule"; clicked => { root.save-schedule(); } }
                    }
                    if root.schedule-kind == "once": HorizontalBox {
                        spacing: 8px;
                        Text { text: "Start:"; vertical-alignment: center; }
                        LineEdit { text <=> root.once-start; placeholder-text: "YYYY-MM-DD HH:MM or Unix"; }
                        Text { text: "Stop:"; vertical-alignment: center; }
                        LineEdit { text <=> root.once-stop; placeholder-text: "Stop date/time (optional)"; }
                    }
                    if root.schedule-kind == "daily": VerticalBox {
                        padding: 0px;
                        spacing: 6px;
                        HorizontalBox {
                            spacing: 8px;
                            Text { text: "Start (HH:MM):"; vertical-alignment: center; }
                            LineEdit { text <=> root.daily-start; placeholder-text: "09:00"; preferred-width: 90px; }
                            Text { text: "Stop (HH:MM):"; vertical-alignment: center; }
                            LineEdit { text <=> root.daily-stop; placeholder-text: "17:00"; preferred-width: 90px; }
                        }
                        HorizontalBox {
                            spacing: 4px;
                            Text { text: "Days:"; vertical-alignment: center; }
                            Button { text: root.day-mon ? "[Mon]" : "Mon"; clicked => { root.day-mon = !root.day-mon; } }
                            Button { text: root.day-tue ? "[Tue]" : "Tue"; clicked => { root.day-tue = !root.day-tue; } }
                            Button { text: root.day-wed ? "[Wed]" : "Wed"; clicked => { root.day-wed = !root.day-wed; } }
                            Button { text: root.day-thu ? "[Thu]" : "Thu"; clicked => { root.day-thu = !root.day-thu; } }
                            Button { text: root.day-fri ? "[Fri]" : "Fri"; clicked => { root.day-fri = !root.day-fri; } }
                            Button { text: root.day-sat ? "[Sat]" : "Sat"; clicked => { root.day-sat = !root.day-sat; } }
                            Button { text: root.day-sun ? "[Sun]" : "Sun"; clicked => { root.day-sun = !root.day-sun; } }
                            Button { text: "All days"; clicked => { root.select-weekdays(true); } }
                            Button { text: "Weekdays"; clicked => { root.select-weekdays(false); } }
                        }
                    }
                    Rectangle { height: 1px; background: #c8d0db; }
                    Text { text: "On Queue Completion"; font-size: 15px; font-weight: 700; color: #143f73; }
                    HorizontalBox {
                        spacing: 6px;
                        Text { text: "Action:"; vertical-alignment: center; }
                        Button { text: root.completion-action == "none" ? "[ None ]" : "None"; clicked => { root.select-completion("none"); } }
                        Button { text: root.completion-action == "notify" ? "[ Notify ]" : "Notify"; clicked => { root.select-completion("notify"); } }
                        Button { text: root.completion-action == "sleep" ? "[ Sleep ]" : "Sleep"; clicked => { root.select-completion("sleep"); } }
                        Button { text: root.completion-action == "hibernate" ? "[ Hibernate ]" : "Hibernate"; clicked => { root.select-completion("hibernate"); } }
                        Button { text: root.completion-action == "shutdown" ? "[ Shutdown ]" : "Shutdown"; clicked => { root.select-completion("shutdown"); } }
                        Button { text: root.completion-action == "restart" ? "[ Restart ]" : "Restart"; clicked => { root.select-completion("restart"); } }
                        Button { text: root.completion-action == "exit" ? "[ Exit ]" : "Exit"; clicked => { root.select-completion("exit"); } }
                        Button { text: root.completion-action == "program" ? "[ Program ]" : "Program"; clicked => { root.select-completion("program"); } }
                    }
                    HorizontalBox {
                        spacing: 8px;
                        Text { text: "Countdown (10–3600 s):"; vertical-alignment: center; }
                        LineEdit { text <=> root.countdown-seconds; placeholder-text: "60"; preferred-width: 80px; }
                        Button { text: "Apply completion action"; clicked => { root.save-completion(); } }
                    }
                    if root.completion-action == "program": VerticalBox {
                        padding: 0px;
                        spacing: 4px;
                        HorizontalBox {
                            spacing: 8px;
                            Text { text: "Executable path:"; vertical-alignment: center; }
                            LineEdit { text <=> root.program-path; placeholder-text: "/path/to/executable"; }
                        }
                        Text { text: "Arguments (one argument per line):"; font-size: 12px; color: #38536f; }
                        TextEdit {
                            text <=> root.program-args;
                            height: 60px;
                        }
                    }
                    if root.is-destructive: HorizontalBox {
                        spacing: 8px;
                        CheckBox {
                            text: root.confirm-label;
                            checked <=> root.confirm-destructive;
                        }
                    }
                    Rectangle { height: 1px; background: #c8d0db; }
                    Text { text: "Recent Queue Events (read-only)"; font-size: 15px; font-weight: 700; color: #143f73; }
                    Rectangle {
                        height: 90px;
                        background: #ffffff;
                        border-width: 1px;
                        border-color: #c8d0db;
                        ListView {
                            for event[index] in root.events: Rectangle {
                                height: 24px;
                                background: Math.mod(index, 2) == 0 ? #ffffff : #f7f9fc;
                                Text { x: 8px; y: 4px; text: event; color: #172a3f; font-size: 12px; }
                            }
                        }
                    }
                }
            }
            Text { text: root.message; wrap: word-wrap; color: #52677c; }
            HorizontalBox {
                spacing: 8px;
                Button { text: "Apply all settings"; clicked => { root.save-all(); } }
                Button { text: "Close"; clicked => { root.dismiss(); } }
            }
        }
    }

    export struct PartialItem {
        label: string,
        checked: bool,
    }

    export component PartialsWindow inherits Window {
        title: "XDM — Clean up partial files";
        width: 760px;
        height: 560px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> directory;
        in property <[PartialItem]> items;
        in property <string> notice;
        in property <bool> confirming;
        in property <string> confirm-text;
        in-out property <string> message: "Choose a folder and scan for .part / .part.meta files that no task owns.";
        callback scan();
        callback toggle(int);
        callback select-all(bool);
        callback delete-selected();
        callback confirm-delete();
        callback cancel-confirm();
        callback dismiss();
        VerticalBox {
            padding: 14px;
            spacing: 8px;
            Text { text: "Unowned partial files"; font-size: 18px; font-weight: 700; color: #143f73; }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Folder:"; vertical-alignment: center; }
                LineEdit { text <=> root.directory; placeholder-text: "/home/user/Downloads"; }
                Button { text: "Scan"; clicked => { root.scan(); } }
            }
            if root.notice != "": Text { text: root.notice; color: #b26a00; wrap: word-wrap; }
            Rectangle {
                background: Palette.color-scheme == ColorScheme.dark ? #21252b : #ffffff;
                border-width: 1px;
                border-color: #c8d0db;
                ListView {
                    for item[index] in root.items: Rectangle {
                        height: 30px;
                        background: item.checked ? #d8eaff : transparent;
                        Text {
                            x: 8px;
                            width: parent.width - 16px;
                            text: (item.checked ? "[x]  " : "[ ]  ") + item.label;
                            overflow: elide;
                            vertical-alignment: center;
                            color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                        }
                        TouchArea { clicked => { root.toggle(index); } }
                    }
                }
            }
            HorizontalBox {
                spacing: 8px;
                Button { text: "Select all"; clicked => { root.select-all(true); } }
                Button { text: "Select none"; clicked => { root.select-all(false); } }
                Button { text: "Delete selected…"; clicked => { root.delete-selected(); } }
            }
            if root.confirming: HorizontalBox {
                spacing: 8px;
                Text { text: root.confirm-text; wrap: word-wrap; color: #b71c1c; vertical-alignment: center; }
                Button { text: "Confirm delete"; clicked => { root.confirm-delete(); } }
                Button { text: "Keep files"; clicked => { root.cancel-confirm(); } }
            }
            Text { text: root.message; wrap: word-wrap; color: #52677c; }
            Button { text: "Close"; clicked => { root.dismiss(); } }
        }
    }

    export component BandwidthWindow inherits Window {
        title: "XDM — Bandwidth";
        width: 760px;
        height: 760px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> global-limit;
        in-out property <string> host-pattern;
        in-out property <string> host-kib;
        in-out property <string> conn-pattern;
        in-out property <string> conn-count;
        in-out property <string> quota-mib;
        in-out property <string> quota-hours: "24";
        in property <[string]> host-limits;
        in property <[string]> host-conns;
        in property <string> status-text;
        in-out property <string> message;
        callback apply-global();
        callback clear-global();
        callback set-host-limit();
        callback remove-host-limit();
        callback pick-host-limit(int);
        callback set-host-conns();
        callback remove-host-conns();
        callback pick-host-conns(int);
        callback apply-quota();
        callback disable-quota();
        callback refresh();
        callback dismiss();
        ScrollView {
            VerticalBox {
                padding: 14px;
                spacing: 8px;
                Text { text: "Bandwidth, host budgets and quota"; font-size: 18px; font-weight: 700; color: #143f73; }
                Text { text: "Global limit over all HTTP downloads"; font-weight: 700; color: #143f73; }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "KiB/s:"; vertical-alignment: center; }
                    LineEdit { text <=> root.global-limit; placeholder-text: "unlimited"; }
                    Button { text: "Apply global limit"; clicked => { root.apply-global(); } }
                    Button { text: "Unlimited"; clicked => { root.clear-global(); } }
                }
                Text { text: "Per-host limits (pattern: example.com, *.example.com or *; 0 KiB/s = explicitly unlimited)"; wrap: word-wrap; font-weight: 700; color: #143f73; }
                VerticalBox {
                    padding: 0px;
                    spacing: 0px;
                    for rule[index] in root.host-limits: Rectangle {
                        height: 26px;
                        background: Math.mod(index, 2) == 0 ? #ffffff : #f7f9fc;
                        Text { x: 8px; width: parent.width - 16px; text: rule; vertical-alignment: center; color: #172a3f; }
                        TouchArea { clicked => { root.pick-host-limit(index); } }
                    }
                    if root.host-limits.length == 0: Text { text: "No host limit rules."; color: #52677c; }
                }
                HorizontalBox {
                    spacing: 8px;
                    LineEdit { text <=> root.host-pattern; placeholder-text: "host pattern"; }
                    LineEdit { text <=> root.host-kib; placeholder-text: "KiB/s"; preferred-width: 90px; }
                    Button { text: "Set limit"; clicked => { root.set-host-limit(); } }
                    Button { text: "Remove rule"; clicked => { root.remove-host-limit(); } }
                }
                Text { text: "Host connection budgets (simultaneous connections per host across all tasks)"; wrap: word-wrap; font-weight: 700; color: #143f73; }
                VerticalBox {
                    padding: 0px;
                    spacing: 0px;
                    for rule[index] in root.host-conns: Rectangle {
                        height: 26px;
                        background: Math.mod(index, 2) == 0 ? #ffffff : #f7f9fc;
                        Text { x: 8px; width: parent.width - 16px; text: rule; vertical-alignment: center; color: #172a3f; }
                        TouchArea { clicked => { root.pick-host-conns(index); } }
                    }
                    if root.host-conns.length == 0: Text { text: "No host budget rules (default budget applies)."; color: #52677c; }
                }
                HorizontalBox {
                    spacing: 8px;
                    LineEdit { text <=> root.conn-pattern; placeholder-text: "host pattern"; }
                    LineEdit { text <=> root.conn-count; placeholder-text: "connections"; preferred-width: 90px; }
                    Button { text: "Set budget"; clicked => { root.set-host-conns(); } }
                    Button { text: "Remove rule"; clicked => { root.remove-host-conns(); } }
                }
                Text { text: "Usage quota (downloads wait in quota_wait once it is used up)"; font-weight: 700; color: #143f73; }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "MiB:"; vertical-alignment: center; }
                    LineEdit { text <=> root.quota-mib; placeholder-text: "off"; preferred-width: 110px; }
                    Text { text: "per hours:"; vertical-alignment: center; }
                    LineEdit { text <=> root.quota-hours; preferred-width: 70px; }
                    Button { text: "Apply quota"; clicked => { root.apply-quota(); } }
                    Button { text: "Turn off"; clicked => { root.disable-quota(); } }
                }
                Text { text: "Status"; font-weight: 700; color: #143f73; }
                Rectangle {
                    background: Palette.color-scheme == ColorScheme.dark ? #21252b : #ffffff;
                    border-width: 1px;
                    border-color: #c8d0db;
                    VerticalBox {
                        padding: 8px;
                        Text { text: root.status-text; wrap: word-wrap; color: #172a3f; }
                    }
                }
                Text { text: root.message; wrap: word-wrap; color: #52677c; }
                HorizontalBox {
                    spacing: 8px;
                    Button { text: "Refresh"; clicked => { root.refresh(); } }
                    Button { text: "Close"; clicked => { root.dismiss(); } }
                }
            }
        }
    }

    export component BatchWindow inherits Window {
        title: "XDM — Add batch download";
        width: 650px;
        height: 440px;
        background: #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> template: "";
        in-out property <string> directory: "";
        in-out property <string> preview-count: "0 URLs";
        in-out property <string> preview-first: "First: —";
        in-out property <string> preview-last: "Last: —";
        in-out property <bool> start-now: true;
        in-out property <string> message: "Enter a URL template such as http://example.com/file[001-010].zip";
        in-out property <bool> can-submit: false;
        callback update-preview();
        callback submit();
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 10px;
            Text { text: "Add batch download"; font-size: 18px; font-weight: 700; color: #143f73; }
            Text { text: "Address / URL template (supports ranges like [1-10], [001-100], [a-z]):"; font-size: 12px; color: #38536f; }
            LineEdit {
                text <=> root.template;
                placeholder-text: "http://example.com/file[001-010].zip";
                edited => { root.update-preview(); }
            }
            Rectangle {
                height: 90px;
                background: #ffffff;
                border-width: 1px;
                border-color: #c8d0db;
                VerticalBox {
                    padding: 8px;
                    spacing: 4px;
                    Text { text: root.preview-count; font-weight: 700; color: #172a3f; }
                    Text { text: root.preview-first; color: #52677c; font-size: 11px; overflow: elide; }
                    Text { text: root.preview-last; color: #52677c; font-size: 11px; overflow: elide; }
                }
            }
            Text { text: "Save to directory:"; font-size: 12px; color: #38536f; }
            LineEdit {
                text <=> root.directory;
                placeholder-text: "/path/to/downloads";
            }
            HorizontalBox {
                padding: 0px;
                Button {
                    text: root.start-now ? "[X] Start queue immediately" : "[  ] Start queue immediately";
                    clicked => { root.start-now = !root.start-now; }
                }
            }
            Text { text: root.message; wrap: word-wrap; color: #52677c; }
            HorizontalBox {
                padding: 0px;
                spacing: 8px;
                Button {
                    text: "Add batch";
                    clicked => { root.update-preview(); root.submit(); }
                }
                Button {
                    text: "Cancel";
                    clicked => { root.dismiss(); }
                }
            }
        }
    }

    export component GrabberWindow inherits Window {
        title: "XDM — Site grabber";
        width: 720px;
        height: 650px;
        background: #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> start-url: "";
        in-out property <string> depth: "2";
        in-out property <string> max-pages: "100";
        in-out property <string> file-types: "zip, pdf, mp4";
        in-out property <string> include-patterns: "";
        in-out property <string> exclude-patterns: "";
        in-out property <string> directory: "";
        in-out property <bool> same-host: true;
        in-out property <bool> obey-robots: true;
        in-out property <bool> enqueue: true;
        in-out property <string> status-text: "Ready to crawl";
        in-out property <bool> is-crawling: false;
        in-out property <string> project-name: "";
        in property <[string]> saved-projects;
        in-out property <int> selected-project: -1;
        in-out property <string> message: "Ready";
        callback start-crawl();
        callback stop-crawl();
        callback save-project();
        callback load-project(int);
        callback remove-project();
        callback refresh-projects();
        callback dismiss();

        ScrollView {
            VerticalBox {
                padding: 16px;
                spacing: 8px;
                Text { text: "Site Grabber"; font-size: 18px; font-weight: 700; color: #143f73; }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Start URL:"; vertical-alignment: center; }
                    LineEdit { text <=> root.start-url; placeholder-text: "http://example.com/"; }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Depth:"; vertical-alignment: center; }
                    LineEdit { text <=> root.depth; preferred-width: 60px; }
                    Text { text: "Max pages:"; vertical-alignment: center; }
                    LineEdit { text <=> root.max-pages; preferred-width: 80px; }
                    Button {
                        text: root.same-host ? "[X] Same host" : "[  ] Same host";
                        clicked => { root.same-host = !root.same-host; }
                    }
                    Button {
                        text: root.obey-robots ? "[X] Robots.txt" : "[  ] Robots.txt";
                        clicked => { root.obey-robots = !root.obey-robots; }
                    }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "File types:"; vertical-alignment: center; }
                    LineEdit { text <=> root.file-types; placeholder-text: "zip, pdf, mp4 (comma-separated)"; }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Include:"; vertical-alignment: center; }
                    LineEdit { text <=> root.include-patterns; placeholder-text: "URL globs to include (*.zip, *files*)"; }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Exclude:"; vertical-alignment: center; }
                    LineEdit { text <=> root.exclude-patterns; placeholder-text: "URL globs to exclude (*thumb*, *preview*)"; }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Save folder:"; vertical-alignment: center; }
                    LineEdit { text <=> root.directory; placeholder-text: "Absolute destination directory"; }
                    Button {
                        text: root.enqueue ? "[X] Start now" : "[  ] Add paused";
                        clicked => { root.enqueue = !root.enqueue; }
                    }
                }
                HorizontalBox {
                    spacing: 8px;
                    Button {
                        text: root.is-crawling ? "Crawling in progress…" : "Start Crawl";
                        enabled: !root.is-crawling;
                        clicked => { root.start-crawl(); }
                    }
                    Button {
                        text: "Stop Crawl";
                        enabled: root.is-crawling;
                        clicked => { root.stop-crawl(); }
                    }
                }
                Rectangle {
                    height: 60px;
                    background: #ffffff;
                    border-width: 1px;
                    border-color: #c8d0db;
                    VerticalBox {
                        padding: 8px;
                        Text { text: "Crawl Progress"; font-weight: 700; color: #143f73; }
                        Text { text: root.status-text; color: #172a3f; font-size: 12px; }
                    }
                }
                Rectangle { height: 1px; background: #c8d0db; }
                Text { text: "Saved Projects"; font-size: 15px; font-weight: 700; color: #143f73; }
                Rectangle {
                    height: 90px;
                    background: #ffffff;
                    border-width: 1px;
                    border-color: #c8d0db;
                    ListView {
                        for proj[index] in root.saved-projects: Rectangle {
                            height: 24px;
                            background: root.selected-project == index ? #d8eaff : (Math.mod(index, 2) == 0 ? #ffffff : #f7f9fc);
                            Text { x: 8px; y: 4px; text: proj; color: #172a3f; font-size: 12px; }
                            TouchArea { clicked => { root.selected-project = index; root.load-project(index); } }
                        }
                    }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Name:"; vertical-alignment: center; }
                    LineEdit { text <=> root.project-name; placeholder-text: "Project name"; }
                    Button { text: "Save"; clicked => { root.save-project(); } }
                    Button { text: "Remove"; clicked => { root.remove-project(); } }
                    Button { text: "Refresh"; clicked => { root.refresh-projects(); } }
                }
                Text { text: root.message; wrap: word-wrap; color: #52677c; }
                HorizontalBox {
                    Button { text: "Close"; clicked => { root.dismiss(); } }
                }
            }
        }
    }

    export component CategoriesWindow inherits Window {
        title: "XDM — Categories";
        width: 720px;
        height: 600px;
        background: #f2f4f7;
        default-font-family: "DejaVu Sans";
        in property <[string]> categories;
        in-out property <int> selected-category: -1;
        in-out property <string> name: "";
        in-out property <string> extensions: "";
        in-out property <string> destination: "";
        in-out property <string> sites: "";
        in-out property <string> message: "Select a category to view or edit, or create a new category.";
        callback select-category(int);
        callback save-category();
        callback remove-category();
        callback clear-fields();
        callback refresh-categories();
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 10px;
            Text { text: "Categories"; font-size: 18px; font-weight: 700; color: #143f73; }
            Text { text: "Defined categories (built-in and custom):"; font-size: 12px; color: #38536f; }
            Rectangle {
                height: 180px;
                background: #ffffff;
                border-width: 1px;
                border-color: #c8d0db;
                ListView {
                    for cat[index] in root.categories: Rectangle {
                        height: 28px;
                        background: root.selected-category == index ? #d8eaff : (Math.mod(index, 2) == 0 ? #ffffff : #f7f9fc);
                        Text { x: 8px; y: 6px; text: cat; color: #172a3f; font-size: 12px; overflow: elide; }
                        TouchArea { clicked => { root.selected-category = index; root.select-category(index); } }
                    }
                }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Name:"; vertical-alignment: center; }
                LineEdit { text <=> root.name; placeholder-text: "Category name (e.g. Archives, Books)"; }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Extensions:"; vertical-alignment: center; }
                LineEdit { text <=> root.extensions; placeholder-text: "zip, rar, 7z (comma-separated)"; }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Destination folder:"; vertical-alignment: center; }
                LineEdit { text <=> root.destination; placeholder-text: "Absolute directory path (empty = default)"; }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Sites:"; vertical-alignment: center; }
                LineEdit { text <=> root.sites; placeholder-text: "*.example.com, myfiles.net"; }
            }
            HorizontalBox {
                spacing: 8px;
                Button { text: "Save / Update"; clicked => { root.save-category(); } }
                Button { text: "Remove"; clicked => { root.remove-category(); } }
                Button { text: "New / Clear"; clicked => { root.clear-fields(); } }
                Button { text: "Refresh"; clicked => { root.refresh-categories(); } }
                Button { text: "Close"; clicked => { root.dismiss(); } }
            }
            Text { text: root.message; wrap: word-wrap; color: #52677c; }
        }
    }

    export component ConnectionWindow inherits Window {
        title: "XDM — Connection settings";
        width: 700px;
        height: 600px;
        background: #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> stall-timeout: "30";
        callback open-bandwidth();
        in-out property <string> proxy-mode: "system";
        in-out property <string> http-proxy: "";
        in-out property <string> https-proxy: "";
        in-out property <string> bypass-list: "";
        in-out property <string> default-connections: "8";
        in-out property <string> message: "Configure proxy routing and default connections.";
        callback select-mode(string);
        callback save-settings();
        callback refresh-settings();
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 10px;
            Text { text: "Connection & Proxy Settings"; font-size: 18px; font-weight: 700; color: #143f73; }
            Text { text: "Proxy Configuration:"; font-size: 14px; font-weight: 700; color: #143f73; }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Mode:"; vertical-alignment: center; }
                Button {
                    text: root.proxy-mode == "system" ? "[ System Proxy ]" : "System Proxy";
                    clicked => { root.select-mode("system"); }
                }
                Button {
                    text: root.proxy-mode == "direct" ? "[ Direct (No Proxy) ]" : "Direct (No Proxy)";
                    clicked => { root.select-mode("direct"); }
                }
                Button {
                    text: root.proxy-mode == "manual" ? "[ Manual Proxy ]" : "Manual Proxy";
                    clicked => { root.select-mode("manual"); }
                }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "HTTP Proxy:"; vertical-alignment: center; }
                LineEdit { text <=> root.http-proxy; placeholder-text: "http://127.0.0.1:8080"; enabled: root.proxy-mode == "manual"; }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "HTTPS Proxy:"; vertical-alignment: center; }
                LineEdit { text <=> root.https-proxy; placeholder-text: "http://127.0.0.1:8080"; enabled: root.proxy-mode == "manual"; }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Bypass list:"; vertical-alignment: center; }
                LineEdit { text <=> root.bypass-list; placeholder-text: "localhost, 127.0.0.1 (comma-separated)"; enabled: root.proxy-mode == "manual"; }
            }
            Rectangle { height: 1px; background: #c8d0db; }
            Text { text: "Download Connections:"; font-size: 14px; font-weight: 700; color: #143f73; }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Default connections for new downloads (1–16):"; vertical-alignment: center; }
                LineEdit { text <=> root.default-connections; preferred-width: 80px; }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "HTTP stall timeout in seconds (1–600):"; vertical-alignment: center; }
                LineEdit { text <=> root.stall-timeout; preferred-width: 80px; }
            }
            HorizontalBox {
                spacing: 8px;
                Button { text: "Bandwidth limits and quota…"; clicked => { root.open-bandwidth(); } }
            }
            Text { text: root.message; wrap: word-wrap; color: #52677c; }
            HorizontalBox {
                spacing: 8px;
                Button { text: "Save settings"; clicked => { root.save-settings(); } }
                Button { text: "Refresh"; clicked => { root.refresh-settings(); } }
                Button { text: "Close"; clicked => { root.dismiss(); } }
            }
        }
    }

    export component AddUrlWindow inherits Window {
        title: "XDM — Add URL";
        width: 620px;
        height: 280px;
        background: #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> url: "";
        in-out property <string> destination: "";
        in-out property <string> message: "Enter URL (http://, https://, or ftp://) and destination file.";
        callback submit();
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 10px;
            Text { text: "Enter new address to download"; font-size: 18px; font-weight: 700; color: #143f73; }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Address:"; vertical-alignment: center; }
                LineEdit { text <=> root.url; placeholder-text: "http://, https://, or ftp://"; }
            }
            HorizontalBox {
                spacing: 8px;
                Text { text: "Save as:"; vertical-alignment: center; }
                LineEdit { text <=> root.destination; placeholder-text: "/path/to/downloaded/file"; }
            }
            Text { text: root.message; wrap: word-wrap; color: #52677c; }
            HorizontalBox {
                spacing: 8px;
                Button { text: "OK / Download"; clicked => { root.submit(); } }
                Button { text: "Cancel"; clicked => { root.dismiss(); } }
            }
        }
    }

    export component PreferencesWindow inherits Window {
        title: "XDM — Preferences";
        width: 720px;
        height: 600px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> theme: "system";
        in-out property <bool> confirm-remove: true;
        in-out property <bool> start-minimized: false;
        in-out property <bool> monitor-clipboard: false;
        in-out property <string> clipboard-extensions: "zip, rar, 7z, exe, msi, iso, mp3, mp4, mkv, pdf";
        in-out property <bool> sound-download-complete: false;
        in-out property <string> sound-download-complete-path: "";
        in-out property <bool> sound-download-failed: false;
        in-out property <string> sound-download-failed-path: "";
        in-out property <bool> sound-queue-complete: false;
        in-out property <string> sound-queue-complete-path: "";
        in-out property <string> language: "en";
        in-out property <string> message: "Configure appearance, sounds, and general options.";

        public function apply-theme(theme: string) {
            if (theme == "dark") {
                Palette.color-scheme = ColorScheme.dark;
            } else if (theme == "light") {
                Palette.color-scheme = ColorScheme.light;
            } else {
                Palette.color-scheme = ColorScheme.unknown;
            }
        }

        callback select-theme(string);
        callback select-language(string);
        callback open-columns();
        callback test-sound(string);
        callback save-preferences();
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 10px;
            Text { text: "Preferences"; font-size: 20px; font-weight: 700; color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73; }
            ScrollView {
                VerticalBox {
                    padding: 0px;
                    spacing: 8px;
                    Text { text: "Appearance & Theme"; font-size: 15px; font-weight: 700; color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73; }
                    HorizontalBox {
                        spacing: 8px;
                        Text { text: "Theme:"; vertical-alignment: center; color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f; }
                        Button {
                            text: root.theme == "system" ? "[ System ]" : "System";
                            clicked => { root.select-theme("system"); }
                        }
                        Button {
                            text: root.theme == "light" ? "[ Light ]" : "Light";
                            clicked => { root.select-theme("light"); }
                        }
                        Button {
                            text: root.theme == "dark" ? "[ Dark ]" : "Dark";
                            clicked => { root.select-theme("dark"); }
                        }
                    }
                    Rectangle { height: 1px; background: Palette.color-scheme == ColorScheme.dark ? #3e4451 : #c8d0db; }
                    Text { text: "General Options"; font-size: 15px; font-weight: 700; color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73; }
                    HorizontalBox {
                        CheckBox {
                            text: "Confirm before removing tasks";
                            checked <=> root.confirm-remove;
                        }
                    }
                    HorizontalBox {
                        CheckBox {
                            text: "Start minimized";
                            checked <=> root.start-minimized;
                        }
                    }
                    Rectangle { height: 1px; background: Palette.color-scheme == ColorScheme.dark ? #3e4451 : #c8d0db; }
                    Text { text: "Clipboard Monitoring"; font-size: 15px; font-weight: 700; color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73; }
                    HorizontalBox {
                        CheckBox {
                            text: "Monitor clipboard for downloadable URLs";
                            checked <=> root.monitor-clipboard;
                        }
                    }
                    HorizontalBox {
                        spacing: 8px;
                        Text { text: "File extensions:"; vertical-alignment: center; color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f; }
                        LineEdit {
                            text <=> root.clipboard-extensions;
                            placeholder-text: "zip, rar, 7z, exe, msi, iso, mp3, mp4, mkv, pdf";
                            enabled: root.monitor-clipboard;
                        }
                    }
                    Rectangle { height: 1px; background: Palette.color-scheme == ColorScheme.dark ? #3e4451 : #c8d0db; }
                    Text { text: "Event Sounds"; font-size: 15px; font-weight: 700; color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73; }
                    VerticalBox {
                        padding: 0px;
                        spacing: 6px;
                        HorizontalBox {
                            spacing: 8px;
                            CheckBox {
                                text: "Download complete:";
                                checked <=> root.sound-download-complete;
                            }
                            LineEdit {
                                text <=> root.sound-download-complete-path;
                                placeholder-text: "Custom sound file path (empty = default)";
                                enabled: root.sound-download-complete;
                            }
                            Button {
                                text: "Test";
                                enabled: root.sound-download-complete;
                                clicked => { root.test-sound("complete"); }
                            }
                        }
                        HorizontalBox {
                            spacing: 8px;
                            CheckBox {
                                text: "Download failed:";
                                checked <=> root.sound-download-failed;
                            }
                            LineEdit {
                                text <=> root.sound-download-failed-path;
                                placeholder-text: "Custom sound file path (empty = default)";
                                enabled: root.sound-download-failed;
                            }
                            Button {
                                text: "Test";
                                enabled: root.sound-download-failed;
                                clicked => { root.test-sound("failed"); }
                            }
                        }
                        HorizontalBox {
                            spacing: 8px;
                            CheckBox {
                                text: "Queue complete:";
                                checked <=> root.sound-queue-complete;
                            }
                            LineEdit {
                                text <=> root.sound-queue-complete-path;
                                placeholder-text: "Custom sound file path (empty = default)";
                                enabled: root.sound-queue-complete;
                            }
                            Button {
                                text: "Test";
                                enabled: root.sound-queue-complete;
                                clicked => { root.test-sound("queue"); }
                            }
                        }
                    }
                    Rectangle { height: 1px; background: Palette.color-scheme == ColorScheme.dark ? #3e4451 : #c8d0db; }
                    Text { text: "Language & Columns"; font-size: 15px; font-weight: 700; color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73; }
                    HorizontalBox {
                        spacing: 8px;
                        Text { text: "Language:"; vertical-alignment: center; color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f; }
                        Button {
                            text: root.language == "en" ? "[ English ]" : "English";
                            clicked => { root.select-language("en"); }
                        }
                        Button {
                            text: root.language == "ur" ? "[ اردو (Urdu) ]" : "اردو (Urdu)";
                            clicked => { root.select-language("ur"); }
                        }
                    }
                    HorizontalBox {
                        Button {
                            text: "Customize columns…";
                            clicked => { root.open-columns(); }
                        }
                    }
                }
            }
            Text { text: root.message; wrap: word-wrap; color: Palette.color-scheme == ColorScheme.dark ? #828997 : #52677c; }
            HorizontalBox {
                spacing: 8px;
                Button { text: "Save preferences"; clicked => { root.save-preferences(); } }
                Button { text: "Close"; clicked => { root.dismiss(); } }
            }
        }
    }

    export component ClipboardPromptWindow inherits Window {
        title: "XDM — Download link detected";
        width: 580px;
        height: 180px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        in-out property <string> url: "";
        callback add-download();
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 10px;
            Text {
                text: "Download link detected in clipboard";
                font-size: 16px;
                font-weight: 700;
                color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73;
            }
            Text {
                text: root.url;
                font-size: 13px;
                color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f;
                overflow: elide;
            }
            HorizontalBox {
                spacing: 8px;
                Button {
                    text: "Add to downloads…";
                    clicked => { root.add-download(); }
                }
                Button {
                    text: "Cancel";
                    clicked => { root.dismiss(); }
                }
            }
        }
    }

    export struct MediaTrackItem {
        text: string,
    }

    export component MediaWindow inherits Window {
        title: "XDM — Add media";
        width: 760px;
        height: 640px;
        background: Palette.color-scheme == ColorScheme.dark ? #1e2227 : #f2f4f7;
        default-font-family: "DejaVu Sans";
        public function apply-theme(theme: string) {
            if (theme == "dark") {
                Palette.color-scheme = ColorScheme.dark;
            } else if (theme == "light") {
                Palette.color-scheme = ColorScheme.light;
            } else {
                Palette.color-scheme = ColorScheme.unknown;
            }
        }
        in-out property <string> url: "";
        in-out property <string> headers: "";
        in-out property <string> destination: "";
        in-out property <string> message: "Enter manifest URL (.m3u8 or .mpd) and click Inspect.";
        in-out property <[MediaTrackItem]> tracks: [];
        in-out property <int> selected-track: -1;
        in-out property <bool> is-live: false;
        in-out property <bool> inspecting: false;
        in-out property <string> mode: "video";
        in-out property <string> video-track: "0";
        in-out property <string> audio-track: "";
        in-out property <string> subtitles-tracks: "";
        in-out property <string> audio-format: "original";
        in-out property <string> audio-bitrate: "128";
        in-out property <string> max-minutes: "";
        in-out property <string> output-container: "mp4";

        callback inspect();
        callback set-mode(string);
        callback set-audio-format(string);
        callback select-track(int);
        callback submit();
        callback dismiss();

        VerticalBox {
            padding: 16px;
            spacing: 8px;
            Text {
                text: "Add media / stream download";
                font-size: 18px;
                font-weight: 700;
                color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73;
            }

            HorizontalBox {
                spacing: 8px;
                Text { text: "Manifest URL:"; vertical-alignment: center; width: 110px; }
                LineEdit { text <=> root.url; placeholder-text: "https://example.com/playlist.m3u8 or .mpd"; }
                Button {
                    text: root.inspecting ? "Inspecting…" : "Inspect";
                    enabled: !root.inspecting;
                    clicked => { root.inspect(); }
                }
            }

            HorizontalBox {
                spacing: 8px;
                Text { text: "Headers (opt):"; vertical-alignment: center; width: 110px; }
                LineEdit { text <=> root.headers; placeholder-text: "Cookie: sid=abc; Referer: https://example.com"; }
            }

            Text {
                text: "Available Tracks:";
                font-weight: 700;
                color: Palette.color-scheme == ColorScheme.dark ? #70a5f5 : #143f73;
            }
            Rectangle {
                height: 120px;
                background: Palette.color-scheme == ColorScheme.dark ? #282c34 : #ffffff;
                border-width: 1px;
                border-color: #c8d0db;
                border-radius: 4px;
                ListView {
                    for track[idx] in root.tracks: Rectangle {
                        height: 28px;
                        background: root.selected-track == idx ? #d8eaff : (Math.mod(idx, 2) == 0 ? (Palette.color-scheme == ColorScheme.dark ? #282c34 : #ffffff) : (Palette.color-scheme == ColorScheme.dark ? #21252b : #f9fafb));
                        Text { x: 8px; y: 6px; text: track.text; color: root.selected-track == idx ? #172a3f : (Palette.color-scheme == ColorScheme.dark ? #abb2bf : #172a3f); font-size: 12px; }
                        TouchArea { clicked => { root.selected-track = idx; root.select-track(idx); } }
                    }
                }
            }

            HorizontalBox {
                spacing: 6px;
                Text { text: "Mode:"; vertical-alignment: center; width: 60px; font-weight: 700; }
                Button { text: root.mode == "video" ? "[ Video ]" : "Video"; clicked => { root.set-mode("video"); } }
                Button { text: root.mode == "audio" ? "[ Audio only ]" : "Audio only"; clicked => { root.set-mode("audio"); } }
                Button { text: root.mode == "subtitles" ? "[ Subtitles only ]" : "Subtitles only"; clicked => { root.set-mode("subtitles"); } }
                Button {
                    text: root.is-live ? (root.mode == "live" ? "[ Live recording ]" : "Live recording") : "Live (disabled)";
                    enabled: root.is-live;
                    clicked => { root.set-mode("live"); }
                }
            }

            if root.mode == "video": VerticalBox {
                spacing: 6px;
                padding: 0px;
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Video track #:"; vertical-alignment: center; width: 110px; }
                    LineEdit { text <=> root.video-track; preferred-width: 80px; }
                    Text { text: "Audio track #:"; vertical-alignment: center; width: 110px; }
                    LineEdit { text <=> root.audio-track; placeholder-text: "opt"; preferred-width: 80px; }
                    Text { text: "Container:"; vertical-alignment: center; }
                    Button { text: root.output-container == "mp4" ? "[ .mp4 ]" : ".mp4"; clicked => { root.output-container = "mp4"; } }
                    Button { text: root.output-container == "mkv" ? "[ .mkv ]" : ".mkv"; clicked => { root.output-container = "mkv"; } }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Subtitles (multi):"; vertical-alignment: center; width: 110px; }
                    LineEdit { text <=> root.subtitles-tracks; placeholder-text: "e.g. 2, 3 (comma-separated track indices)"; }
                }
            }

            if root.mode == "audio": VerticalBox {
                spacing: 6px;
                padding: 0px;
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Audio track #:"; vertical-alignment: center; width: 110px; }
                    LineEdit { text <=> root.audio-track; placeholder-text: "track index"; preferred-width: 80px; }
                    Text { text: "Format:"; vertical-alignment: center; }
                    Button { text: root.audio-format == "original" ? "[ Original ]" : "Original"; clicked => { root.set-audio-format("original"); } }
                    Button { text: root.audio-format == "mp3" ? "[ MP3 ]" : "MP3"; clicked => { root.set-audio-format("mp3"); } }
                    Button { text: root.audio-format == "m4a" ? "[ M4A ]" : "M4A"; clicked => { root.set-audio-format("m4a"); } }
                    Button { text: root.audio-format == "opus" ? "[ Opus ]" : "Opus"; clicked => { root.set-audio-format("opus"); } }
                }
                HorizontalBox {
                    spacing: 8px;
                    Text { text: "Bitrate (kbps):"; vertical-alignment: center; width: 110px; }
                    LineEdit {
                        text <=> root.audio-bitrate;
                        enabled: root.audio-format != "original";
                        placeholder-text: "32–320 (e.g. 128)";
                        preferred-width: 120px;
                    }
                }
            }

            if root.mode == "subtitles": HorizontalBox {
                spacing: 8px;
                Text { text: "Subtitle track #:"; vertical-alignment: center; width: 110px; }
                LineEdit { text <=> root.subtitles-tracks; placeholder-text: "track index (e.g. 2)"; preferred-width: 100px; }
            }

            if root.mode == "live": HorizontalBox {
                spacing: 8px;
                Text { text: "Video track #:"; vertical-alignment: center; width: 110px; }
                LineEdit { text <=> root.video-track; preferred-width: 80px; }
                Text { text: "Audio track #:"; vertical-alignment: center; width: 110px; }
                LineEdit { text <=> root.audio-track; preferred-width: 80px; }
                Text { text: "Max minutes:"; vertical-alignment: center; width: 90px; }
                LineEdit { text <=> root.max-minutes; placeholder-text: "opt (e.g. 60)"; preferred-width: 100px; }
            }

            HorizontalBox {
                spacing: 8px;
                Text { text: "Save as:"; vertical-alignment: center; width: 110px; }
                LineEdit { text <=> root.destination; placeholder-text: "/path/to/output.file"; }
            }

            Text { text: root.message; wrap: word-wrap; color: Palette.color-scheme == ColorScheme.dark ? #abb2bf : #52677c; }

            HorizontalBox {
                spacing: 8px;
                Button { text: "Download"; clicked => { root.submit(); } }
                Button { text: "Cancel"; clicked => { root.dismiss(); } }
            }
        }
    }
}

fn task_details(task: &TaskSummary) -> String {
    format!(
        "Queue #{}\nTask #{}\nStatus: {}\nCategory: {}\nType: {}\nPriority: {}\nConnections: {}\n\nSource URL: {}\n\nDestination: {}\n\nDownloaded: {} bytes\nTotal: {}\nSpeed cap: {}\nCreated (UTC): {}\nUpdated (UTC): {}\nVideo track: {:?}\nAudio track: {:?}\n\nLast error: {}\nRestart reason: {}",
        task.queue_id,
        task.id,
        task.status,
        task.category,
        task.kind,
        task.priority,
        task.connections,
        task.url,
        task.output,
        task.downloaded.unwrap_or(0),
        task.total
            .map_or_else(|| "Unknown".into(), |total| format!("{total} bytes")),
        task.rate_limit.map_or_else(
            || "Unlimited".into(),
            |rate| format!("{} KiB/s", rate / 1024)
        ),
        task.created_at,
        task.updated_at,
        task.media_video,
        task.media_audio,
        task.error.as_deref().unwrap_or("None"),
        task.restart_reason.as_deref().unwrap_or("None")
    )
}

fn request(paths: &Paths, command: Command) -> String {
    match paths.request(command) {
        Ok(reply) if reply.ok => reply
            .task_id
            .map_or_else(|| "Ready".into(), |id| format!("Task {id} accepted")),
        Ok(reply) => reply
            .error
            .unwrap_or_else(|| "Daemon rejected request".into()),
        Err(error) => format!("Daemon unavailable: {error}"),
    }
}

/// Task id at table row `index`; 0 marks a row whose page is not loaded yet.
fn id_at(ids: &[i64], index: usize) -> Option<i64> {
    ids.get(index).copied().filter(|id| *id > 0)
}

fn selected_task(ui: &MainWindow, ids: &Arc<Mutex<Vec<i64>>>) -> Option<i64> {
    let task_id = usize::try_from(ui.get_selected())
        .ok()
        .and_then(|index| ids.lock().ok().and_then(|guard| id_at(&guard, index)));
    if task_id.is_none() {
        ui.set_message("Select a task first".into());
    }
    task_id
}

/// Ids the current action applies to: the multi-selection (in list order, loaded
/// rows only) or, failing that, the single highlighted row.
fn target_ids(
    ui: &MainWindow,
    ids: &Arc<Mutex<Vec<i64>>>,
    set: &Arc<Mutex<std::collections::HashSet<usize>>>,
) -> Vec<i64> {
    let ids = match ids.lock() {
        Ok(guard) => guard,
        Err(_) => return Vec::new(),
    };
    let set = set.lock().map(|s| s.clone()).unwrap_or_default();
    let multi: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|id| *id > 0 && set.contains(&(*id as usize)))
        .collect();
    if !multi.is_empty() {
        return multi;
    }
    usize::try_from(ui.get_selected())
        .ok()
        .and_then(|i| id_at(&ids, i))
        .into_iter()
        .collect()
}

/// "Select all" meaning every task matching the filter that was shown, not
/// only the loaded rows; `total` is what the user saw.
#[derive(Clone)]
struct AllMatching {
    filter: xdm_ipc::TaskFilter,
    total: u64,
}

type AllSel = Arc<Mutex<Option<AllMatching>>>;

/// What a toolbar action applies to.
#[derive(Clone)]
enum Target {
    Ids(Vec<i64>),
    All(AllMatching),
}

impl Target {
    fn count(&self) -> usize {
        match self {
            Target::Ids(ids) => ids.len(),
            Target::All(all) => all.total as usize,
        }
    }
}

fn current_filter(ui: &MainWindow) -> xdm_ipc::TaskFilter {
    build_query(ui, &("id".to_string(), true)).filter
}

/// The selection an action should use: all-matching mode while the filter is
/// still the one that was selected, otherwise the row selection.
fn resolve_target(
    ui: &MainWindow,
    ids: &Arc<Mutex<Vec<i64>>>,
    set: &Arc<Mutex<std::collections::HashSet<usize>>>,
    all: &AllSel,
) -> Target {
    if let Ok(mut guard) = all.lock() {
        match guard.as_ref() {
            Some(sel) if sel.filter == current_filter(ui) => return Target::All(sel.clone()),
            Some(_) => *guard = None,
            None => {}
        }
    }
    Target::Ids(target_ids(ui, ids, set))
}

fn bulk_command(all: &AllMatching, action: xdm_ipc::BulkAction) -> Command {
    Command::TaskBulk {
        action,
        selection: xdm_ipc::BulkSelection::Filter(all.filter.clone()),
        expected_total: Some(all.total),
    }
}

/// One-line outcome of a `TaskBulk` reply.
fn bulk_message(result: Result<xdm_ipc::Reply, String>, verb: &str) -> String {
    match result {
        Ok(reply) if reply.ok => {
            let report = reply.bulk.unwrap_or_default();
            if report.failed == 0 {
                format!("{verb} {} task(s)", report.done)
            } else {
                let first = report.failures.first().map(|f| format!(" (#{}: {})", f.task_id, f.error)).unwrap_or_default();
                format!("{verb} {} task(s); {} could not be changed{first}", report.done, report.failed)
            }
        }
        Ok(reply) => reply.error.unwrap_or_else(|| "Daemon rejected request".into()),
        Err(error) => format!("Daemon unavailable: {error}"),
    }
}

/// Removes `target` on the daemon; the message is the user-facing outcome.
fn run_remove(
    paths: &Paths,
    target: &Target,
    partial: &std::collections::HashSet<i64>,
    partial_all: bool,
) -> Result<String, String> {
    match target {
        Target::All(all) => {
            let action = if partial_all {
                xdm_ipc::BulkAction::RemoveWithPartials
            } else {
                xdm_ipc::BulkAction::Remove
            };
            let reply = paths.request(bulk_command(all, action)).map_err(|e| e.to_string())?;
            if !reply.ok {
                return Err(reply.error.unwrap_or_else(|| "Remove rejected".into()));
            }
            let report = reply.bulk.unwrap_or_default();
            if report.failed > 0 {
                let first = report.failures.first().map(|f| format!(" (#{}: {})", f.task_id, f.error)).unwrap_or_default();
                Err(format!("{} removed; {} could not be removed{first}", report.done, report.failed))
            } else {
                Ok(format!("{} tasks removed", report.done))
            }
        }
        Target::Ids(targets) => {
            for task_id in targets {
                let command = if partial.contains(task_id) {
                    Command::TaskRemoveEx { task_id: *task_id, delete_partial: true }
                } else {
                    Command::Remove { task_id: *task_id }
                };
                match paths.request(command) {
                    Ok(reply) if reply.ok => {}
                    Ok(reply) => return Err(reply.error.unwrap_or_else(|| "Remove rejected".into())),
                    Err(error) => return Err(error.to_string()),
                }
            }
            Ok(if targets.len() == 1 {
                format!("Task #{} removed; files kept", targets[0])
            } else {
                format!("{} tasks removed; files kept", targets.len())
            })
        }
    }
}

/// Runs `f` on the paged table model behind the main window.
fn with_model<R>(ui: &MainWindow, f: impl FnOnce(&PagedModel<TaskRow>) -> R) -> Option<R> {
    let model_rc = ui.get_tasks();
    let model = model_rc.as_any().downcast_ref::<PagedModel<TaskRow>>()?;
    Some(f(model))
}

/// Push the selection flags of `set` into the loaded model rows (only changed rows).
fn sync_selection(ui: &MainWindow, set: &std::collections::HashSet<usize>) {
    with_model(ui, |model| {
        model.update_cached(|row| {
            let want = row.id > 0 && set.contains(&(row.id as usize));
            let changed = row.selected != want;
            row.selected = want;
            changed
        });
    });
}

/// Off-UI thread IPC worker keeping the interface responsive.
#[derive(Clone)]
struct IpcClient {
    tx: Sender<Box<dyn FnOnce(&Paths) + Send>>,
}

impl IpcClient {
    fn new(paths: Paths) -> Self {
        let (tx, rx) = channel::<Box<dyn FnOnce(&Paths) + Send>>();
        std::thread::spawn(move || {
            while let Ok(job) = rx.recv() {
                job(&paths);
            }
        });
        Self { tx }
    }

    fn execute<F>(&self, job: F)
    where
        F: FnOnce(&Paths) + Send + 'static,
    {
        let _ = self.tx.send(Box::new(job));
    }
}

/// Runs `work` on the IPC worker thread and `done` on the UI thread, where it
/// may use non-`Send` window state. Keeps dialogs from freezing on the daemon.
fn ipc_then<T: Send + 'static>(
    ipc: &IpcClient,
    work: impl FnOnce(&Paths) -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    struct Shot<T> {
        value: Option<T>,
        waker: Option<std::task::Waker>,
    }
    struct Wait<T>(Arc<Mutex<Shot<T>>>);
    impl<T> std::future::Future for Wait<T> {
        type Output = Option<T>;
        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<T>> {
            let Ok(mut shot) = self.0.lock() else {
                return std::task::Poll::Ready(None);
            };
            match shot.value.take() {
                Some(value) => std::task::Poll::Ready(Some(value)),
                None => {
                    shot.waker = Some(cx.waker().clone());
                    std::task::Poll::Pending
                }
            }
        }
    }
    let shot = Arc::new(Mutex::new(Shot { value: None, waker: None }));
    let sender = shot.clone();
    ipc.execute(move |paths| {
        let value = work(paths);
        let waker = sender.lock().ok().and_then(|mut shot| {
            shot.value = Some(value);
            shot.waker.take()
        });
        if let Some(waker) = waker {
            waker.wake();
        }
    });
    let _ = slint::spawn_local(async move {
        if let Some(value) = Wait(shot).await {
            done(value);
        }
    });
}

/// `GetTask` result as `Ok(task)` or a user-facing message.
fn fetch_task(paths: &Paths, task_id: i64) -> Result<TaskSummary, String> {
    match paths.request(Command::GetTask { task_id }) {
        Ok(reply) if reply.ok => reply.tasks.into_iter().next().ok_or_else(|| "Task not found".to_owned()),
        Ok(reply) => Err(reply.error.unwrap_or_else(|| "Task not found".into())),
        Err(error) => Err(error.to_string()),
    }
}

fn is_live_task(
    db_path: &std::path::Path,
    task_id: i64,
    media_seconds: Option<u64>,
    live_ids: &Arc<Mutex<std::collections::HashSet<i64>>>,
) -> bool {
    if media_seconds.is_some() {
        return true;
    }
    if let Ok(guard) = live_ids.lock() {
        if guard.contains(&task_id) {
            return true;
        }
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    let mode_res: rusqlite::Result<String> = conn.query_row(
        "SELECT media_mode_json FROM tasks WHERE id = ?1",
        [task_id],
        |row| row.get(0),
    );
    if let Ok(json) = mode_res {
        if let Ok(mode) = serde_json::from_str::<xdm_ipc::MediaMode>(&json) {
            if matches!(mode, xdm_ipc::MediaMode::Live { .. }) {
                if let Ok(mut guard) = live_ids.lock() {
                    guard.insert(task_id);
                }
                return true;
            }
        }
    }
    false
}

fn loading_row() -> TaskRow {
    TaskRow {
        id: 0,
        filename: "Loading…".into(),
        size: "".into(),
        status: "".into(),
        time_left: "".into(),
        transfer_rate: "".into(),
        last_try_date: "".into(),
        description: "".into(),
        category: "".into(),
        queue: "".into(),
        progress: 0.0,
        selected: false,
    }
}

fn row_from_summary(
    task: xdm_ipc::TaskSummary,
    queue_map: &std::collections::HashMap<i64, String>,
    selected_set: &std::collections::HashSet<usize>,
) -> TaskRow {
    let progress = match (task.downloaded, task.total) {
        (Some(done), Some(total)) if total > 0 => (done.saturating_mul(100) / total).min(100),
        _ => 0,
    };
    let is_selected = selected_set.contains(&(task.id as usize));
    let q_name = queue_map.get(&task.queue_id).cloned().unwrap_or_else(|| {
        if task.queue_id == 1 {
            "Default".into()
        } else {
            format!("#{}", task.queue_id)
        }
    });
    TaskRow {
        id: task.id as i32,
        filename: task_table::format_filename(&task.output, &task.url).into(),
        size: task_table::format_size(task.downloaded, task.total).into(),
        status: task_table::format_status(&task.status, task.downloaded, task.total).into(),
        time_left: task_table::format_time_left(&task.status, task.eta_seconds).into(),
        transfer_rate: task_table::format_rate(task.bytes_per_second).into(),
        last_try_date: task_table::format_date(&task.updated_at, &task.created_at).into(),
        description: (if !task.output.is_empty() { task.output } else { task.url }).into(),
        category: task.category.into(),
        queue: q_name.into(),
        progress: progress as f32,
        selected: is_selected,
    }
}

/// Daemon sort key for a header column. Time left sorts by progress, the
/// closest stored value to remaining work.
fn daemon_sort(column: &str, asc: bool, queue_filtered: bool) -> Option<String> {
    let key = match column {
        "id" if queue_filtered => "position",
        "id" => "id",
        "filename" | "description" => "output",
        "status" => "status",
        "category" => "category",
        "last_try_date" => "updated",
        "size" => "size",
        "time_left" => "progress",
        "transfer_rate" => "rate",
        "queue" => "queue",
        _ => return None,
    };
    Some(format!("{}{key}", if asc { "" } else { "-" }))
}

/// Builds the daemon query from the window state. `status:<name>` words in the
/// search box select a status; everything else is the URL/file-name search.
fn build_query(ui: &MainWindow, sort: &(String, bool)) -> Query {
    let queue_id = ui.get_queue_filter_id().as_str().parse::<i64>().unwrap_or(0);
    let category = ui.get_category_filter().to_string();
    let mut status = None;
    let mut words = Vec::new();
    for word in ui.get_search_query().to_string().split_whitespace() {
        match word.strip_prefix("status:") {
            Some(value) if !value.is_empty() => status = Some(value.to_lowercase()),
            _ => words.push(word.to_owned()),
        }
    }
    let search = words.join(" ");
    Query {
        sort: daemon_sort(&sort.0, sort.1, queue_id != 0).unwrap_or_else(|| "id".into()),
        filter: xdm_ipc::TaskFilter {
            status,
            category: (category != "All").then_some(category),
            queue_id: (queue_id != 0).then_some(queue_id),
            search: (!search.is_empty()).then_some(search),
        },
    }
}

struct PageResult {
    pages: Vec<(usize, Vec<TaskSummary>)>,
    total: usize,
}

/// Fetches whole pages with `ListPage`; the daemon sorts and filters. Replies
/// may reach 16 MiB, which `Paths::request` already reads with `read_frame_max`.
fn fetch_pages(paths: &Paths, query: &Query, pages: &[usize]) -> Result<PageResult, String> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for &page in pages {
        let reply = paths
            .request(Command::ListPage {
                offset: (page * PAGE) as u64,
                limit: PAGE as u64,
                sort: Some(query.sort.clone()),
                filter: Some(query.filter.clone()),
            })
            .map_err(|error| format!("Daemon unavailable: {error}"))?;
        if !reply.ok {
            return Err(reply.error.unwrap_or_else(|| "List failed".into()));
        }
        total = reply.total as usize;
        out.push((page, reply.tasks));
    }
    Ok(PageResult { pages: out, total })
}

fn queue_state_text(reply: &xdm_ipc::Reply) -> String {
    let state = if let Some(at) = reply.queue_start_at {
        Local
            .timestamp_opt(at, 0)
            .single()
            .map(|time| format!("Queue scheduled for {}", time.format("%Y-%m-%d %H:%M")))
            .unwrap_or_else(|| "Queue scheduled".into())
    } else if reply.queue_running == Some(true) {
        "Queue running".into()
    } else {
        "Queue stopped".into()
    };
    if let Some(at) = reply.queue_stop_at {
        let stop = Local
            .timestamp_opt(at, 0)
            .single()
            .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| at.to_string());
        format!("{state} · stops {stop}")
    } else {
        state
    }
}

/// State shared by the table refresh, the page-miss fetcher and the actions.
#[derive(Clone)]
struct TableShared {
    queues: Arc<Mutex<std::collections::HashMap<i64, String>>>,
    selected: Arc<Mutex<std::collections::HashSet<usize>>>,
    ids: Arc<Mutex<Vec<i64>>>,
    /// Raw daemon status and live-recording seconds of every loaded task.
    statuses: Arc<Mutex<std::collections::HashMap<i64, (String, Option<u64>)>>>,
    /// Set while "all matching the filter" is selected.
    all: AllSel,
}

impl TableShared {
    /// `XDM_GUI_PERF`: renders the window once (software renderer snapshot) and logs
    /// how long the page took from request to pixels.
    fn log_render(ui: &MainWindow, label: &str, started: std::time::Instant) {
        let render_start = std::time::Instant::now();
        let rendered = ui.window().take_snapshot().is_ok();
        eprintln!(
            "[xdm-gui] {label} render={:?} fetch_to_render={:?} snapshot_ok={rendered}",
            render_start.elapsed(),
            started.elapsed()
        );
    }

    fn status_of(&self, id: i64) -> Option<String> {
        self.statuses.lock().ok().and_then(|map| map.get(&id).map(|entry| entry.0.clone()))
    }

    /// UI thread: converts fetched summaries to rows, installs them into the
    /// model and refreshes the row-index to task-id mirror.
    fn install(&self, ui: &MainWindow, epoch: u64, result: PageResult, replace_all: bool) -> bool {
        let queues = self.queues.lock().map(|g| g.clone()).unwrap_or_default();
        let selected = self.selected.lock().map(|g| g.clone()).unwrap_or_default();
        let all_selected = self.all.lock().map(|g| g.is_some()).unwrap_or(false);
        if let Ok(mut statuses) = self.statuses.lock() {
            if statuses.len() > 50_000 {
                statuses.clear();
            }
            for (_, tasks) in &result.pages {
                for task in tasks {
                    statuses.insert(task.id, (task.status.clone(), task.media_seconds));
                }
            }
        }
        let total = result.total;
        let pages = result
            .pages
            .into_iter()
            .map(|(page, tasks)| {
                let rows = tasks
                    .into_iter()
                    .map(|task| {
                        let mut row = row_from_summary(task, &queues, &selected);
                        row.selected |= all_selected;
                        row
                    })
                    .collect();
                (page, rows)
            })
            .collect();
        with_model(ui, |model| {
            let applied = model.apply(epoch, total, pages, replace_all);
            if applied {
                if let Ok(mut ids) = self.ids.lock() {
                    *ids = model.ids(|row| i64::from(row.id));
                }
            }
            applied
        })
        .unwrap_or(false)
    }
}

macro_rules! install_tr {
    ($win:expr) => {{
        let tr = $win.global::<Tr>();
        tr.on_t(|text, _lang| i18n::translate(text.as_str()).into());
        tr.set_lang(i18n::current_language().into());
    }};
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    xdm_ipc::ops::handle_version_flag("xdm-gui");
    let paths = Paths::open()?;
    xdm_ipc::ops::init_logging(&paths.dir, "xdm-gui");
    xdm_ipc::ops::install_panic_hook(&paths.dir, "xdm-gui");
    let ipc = IpcClient::new(paths.clone());
    let ui = MainWindow::new()?;
    let task_ids = Arc::new(Mutex::new(Vec::new()));
    let live_task_ids = Arc::new(Mutex::new(std::collections::HashSet::<i64>::new()));
    let gui_settings = Arc::new(Mutex::new(settings::load_settings(&paths.dir)));

    let initial_settings = gui_settings.lock().map(|s| s.clone()).unwrap_or_default();
    ui.invoke_apply_theme(initial_settings.theme.clone().into());
    if initial_settings.start_minimized {
        ui.window().set_minimized(true);
    }
    i18n::set_language(&initial_settings.language);
    install_tr!(ui);
    ui.set_col_filename(initial_settings.columns.filename);
    ui.set_col_size(initial_settings.columns.size);
    ui.set_col_status(initial_settings.columns.status);
    ui.set_col_time_left(initial_settings.columns.time_left);
    ui.set_col_transfer_rate(initial_settings.columns.transfer_rate);
    ui.set_col_last_try_date(initial_settings.columns.last_try_date);
    ui.set_col_description(initial_settings.columns.description);
    ui.set_col_category(initial_settings.columns.category);
    ui.set_col_queue(initial_settings.columns.queue);

    let sort_state = Arc::new(Mutex::new(("id".to_string(), true)));
    let selected_task_ids = Arc::new(Mutex::new(std::collections::HashSet::<usize>::new()));
    let tasks_model = Rc::new(PagedModel::<TaskRow>::new(loading_row()));
    ui.set_tasks(ModelRc::from(tasks_model.clone()));
    let perf_enabled = std::env::var_os("XDM_GUI_PERF").is_some();
    let table_shared = TableShared {
        queues: Arc::new(Mutex::new(std::collections::HashMap::new())),
        selected: selected_task_ids.clone(),
        ids: task_ids.clone(),
        statuses: Arc::new(Mutex::new(std::collections::HashMap::new())),
        all: Arc::new(Mutex::new(None)),
    };
    {
        // A rendered row on an uncached page fetches just that page.
        let ipc = ipc.clone();
        let weak = ui.as_weak();
        let shared = table_shared.clone();
        let fetch: FetchPage = Rc::new(move |page, epoch, query| {
            let weak = weak.clone();
            let shared = shared.clone();
            ipc.execute(move |paths| {
                let started = std::time::Instant::now();
                let result = fetch_pages(paths, &query, &[page]);
                let ipc_time = started.elapsed();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return; };
                    match result {
                        Ok(result) => {
                            let rows = result.pages.first().map_or(0, |(_, tasks)| tasks.len());
                            let installed = std::time::Instant::now();
                            if shared.install(&ui, epoch, result, false) && perf_enabled {
                                let label = format!(
                                    "page-fetch page={page} rows={rows} ipc={ipc_time:?} install={:?} fetch_to_install={:?}",
                                    installed.elapsed(),
                                    started.elapsed()
                                );
                                TableShared::log_render(&ui, &label, started);
                            }
                        }
                        Err(error) => {
                            with_model(&ui, |model| model.fetch_failed(epoch, page));
                            ui.set_message(error.into());
                        }
                    }
                });
            });
        });
        tasks_model.set_fetch(fetch);
    }

    let previous_task_statuses = Arc::new(Mutex::new(std::collections::HashMap::<i64, String>::new()));
    let previous_queue_statuses = Arc::new(Mutex::new(std::collections::HashMap::<i64, bool>::new()));
    let initialized_statuses = Arc::new(AtomicBool::new(false));

    let weak_completion = ui.as_weak();
    let ipc_completion = ipc.clone();
    let polling_completion = Arc::new(AtomicBool::new(false));
    let is_polling_comp = polling_completion.clone();
    let has_pending_action = Arc::new(AtomicBool::new(false));
    let has_pending_flag = has_pending_action.clone();
    let poll_completion: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if is_polling_comp.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = weak_completion.clone();
        let is_polling = is_polling_comp.clone();
        let has_pending = has_pending_flag.clone();
        ipc_completion.execute(move |paths| {
            let reply_res = paths.request(Command::CompletionStatus);
            let _ = slint::invoke_from_event_loop(move || {
                is_polling.store(false, Ordering::SeqCst);
                let Some(ui) = weak.upgrade() else { return; };
                if let Ok(reply) = reply_res {
                    if !reply.completion_pending.is_empty() {
                        has_pending.store(true, Ordering::SeqCst);
                        if let Some((banner_text, q_id, secs)) = scheduler::format_countdown_banner(&reply.completion_pending) {
                            ui.set_has_pending_completion(true);
                            ui.set_pending_action_text(banner_text.into());
                            ui.set_pending_queue_id(q_id as i32);
                            ui.set_pending_seconds_remaining(secs as i32);
                        }
                    } else {
                        has_pending.store(false, Ordering::SeqCst);
                        ui.set_has_pending_completion(false);
                        ui.set_pending_action_text("".into());
                    }
                }
            });
        });
    });

    let weak_main_cancel = ui.as_weak();
    let ipc_cancel = ipc.clone();
    let poll_cancel = poll_completion.clone();
    ui.on_cancel_completion(move |queue_id| {
        let weak = weak_main_cancel.clone();
        let poll = poll_cancel.clone();
        ipc_cancel.execute(move |paths| {
            let res = request(paths, Command::CompletionCancel { queue_id: queue_id as i64 });
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(format!("Cancellation: {res}").into());
                }
                poll();
            });
        });
    });

    let weak = ui.as_weak();
    let ipc_refresh = ipc.clone();
    let refreshing = Arc::new(AtomicBool::new(false));
    let refresh_again = Arc::new(AtomicBool::new(false));
    let refresh_tick = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let refresh_slot: Arc<std::sync::OnceLock<Arc<dyn Fn() + Send + Sync>>> = Arc::new(std::sync::OnceLock::new());
    let prev_tasks_refresh = previous_task_statuses.clone();
    let prev_queues_refresh = previous_queue_statuses.clone();
    let initialized_statuses_refresh = initialized_statuses.clone();
    let settings_sound_refresh = gui_settings.clone();
    let live_ids_refresh = live_task_ids.clone();
    let db_path_refresh = paths.dir.join("tasks.sqlite3");
    let sort_state_refresh = sort_state.clone();
    let shared_refresh = table_shared.clone();
    let refresh: Arc<dyn Fn() + Send + Sync> = {
        let refresh_slot = refresh_slot.clone();
        Arc::new(move || {
            if refreshing.swap(true, Ordering::SeqCst) {
                // Sort/filter changes made while a refresh is in flight must not be lost.
                refresh_again.store(true, Ordering::SeqCst);
                return;
            }
            let weak = weak.clone();
            let ipc_refresh = ipc_refresh.clone();
            let refreshing = refreshing.clone();
            let refresh_again = refresh_again.clone();
            let refresh_tick = refresh_tick.clone();
            let refresh_slot = refresh_slot.clone();
            let prev_tasks = prev_tasks_refresh.clone();
            let prev_queues = prev_queues_refresh.clone();
            let initialized_statuses = initialized_statuses_refresh.clone();
            let sound_settings = settings_sound_refresh.clone();
            let live_ids_refresh = live_ids_refresh.clone();
            let db_path_refresh = db_path_refresh.clone();
            let sort_state = sort_state_refresh.clone();
            let shared = shared_refresh.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else {
                    refreshing.store(false, Ordering::SeqCst);
                    return;
                };
                let sort = sort_state.lock().map(|s| s.clone()).unwrap_or(("id".into(), true));
                let query = build_query(&ui, &sort);
                // Re-fetch exactly the pages the table rendered since the last refresh.
                let Some((epoch, pages)) = with_model(&ui, |model| {
                    model.begin_query(&query);
                    let mut pages = model.take_touched();
                    if pages.is_empty() {
                        pages.push(0);
                    }
                    (model.epoch(), pages)
                }) else {
                    refreshing.store(false, Ordering::SeqCst);
                    return;
                };
                let previous_selection = usize::try_from(ui.get_selected())
                    .ok()
                    .and_then(|index| shared.ids.lock().ok().and_then(|ids| id_at(&ids, index)));
                // The cheap queue snapshot rides on QueueEvents; the 1000-row List
                // (queue run state, schedule) is only needed now and then.
                let want_state = refresh_tick.fetch_add(1, Ordering::SeqCst) % 5 == 0;
                let queue_filter = query.filter.queue_id.unwrap_or(0);
                let sort_key = query.sort.clone();
                ipc_refresh.execute(move |paths| {
                    let started = std::time::Instant::now();
                    let pages_res = fetch_pages(paths, &query, &pages);
                    let ipc_done = std::time::Instant::now();
                    let queues_res = paths
                        .request(Command::QueueEvents { queue_id: 1 })
                        .ok()
                        .filter(|reply| reply.ok)
                        .map(|reply| reply.queues);
                    let state_res = if want_state {
                        paths.request(Command::List).ok().filter(|reply| reply.ok)
                    } else {
                        None
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        let ui_start = std::time::Instant::now();
                        refreshing.store(false, Ordering::SeqCst);
                        let Some(ui) = weak.upgrade() else {
                            return;
                        };
                        match pages_res {
                            Ok(result) => {
                                if let Some(queues) = &queues_res {
                                    if let Ok(mut map) = shared.queues.lock() {
                                        *map = queues.iter().map(|q| (q.id, q.name.clone())).collect();
                                    }
                                }
                                if let Some(reply) = &state_res {
                                    ui.set_queue_state(queue_state_text(reply).into());
                                }
                                if let (Ok(mut pt), Ok(mut pq)) = (prev_tasks.lock(), prev_queues.lock()) {
                                    let is_first = !initialized_statuses.swap(true, Ordering::SeqCst);
                                    if !is_first {
                                        if let Ok(settings_guard) = sound_settings.lock() {
                                            for (_, tasks) in &result.pages {
                                                for task in tasks {
                                                    if let Some(old_status) = pt.get(&task.id) {
                                                        if !task_table::is_complete(old_status) && task_table::is_complete(&task.status) {
                                                            sound::trigger_event_sound(&settings_guard, sound::SoundEvent::DownloadComplete);
                                                        } else if old_status != "failed" && task.status == "failed" {
                                                            sound::trigger_event_sound(&settings_guard, sound::SoundEvent::DownloadFailed);
                                                        }
                                                    }
                                                }
                                            }
                                            for queue in queues_res.iter().flatten() {
                                                if let Some(was_running) = pq.get(&queue.id) {
                                                    if *was_running && !queue.running {
                                                        sound::trigger_event_sound(&settings_guard, sound::SoundEvent::QueueComplete);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if pt.len() > 50_000 {
                                        pt.clear();
                                    }
                                    for (_, tasks) in &result.pages {
                                        for task in tasks {
                                            pt.insert(task.id, task.status.clone());
                                        }
                                    }
                                    for queue in queues_res.iter().flatten() {
                                        pq.insert(queue.id, queue.running);
                                    }
                                }
                                let queue_caption = if queue_filter == 0 {
                                    "All queues".to_owned()
                                } else {
                                    shared
                                        .queues
                                        .lock()
                                        .ok()
                                        .and_then(|map| map.get(&queue_filter).cloned())
                                        .unwrap_or_else(|| "Queue unavailable".into())
                                };
                                ui.set_queue_caption(queue_caption.into());
                                let rows: usize = result.pages.iter().map(|(_, tasks)| tasks.len()).sum();
                                let total = result.total;
                                let diff_start = std::time::Instant::now();
                                let installed = shared.install(&ui, epoch, result, true);
                                if installed && std::env::var_os("XDM_GUI_PERF").is_some() {
                                    let label = format!(
                                        "refresh pages={} rows={rows} total={total} sort={sort_key} ipc={:?} ui_thread_total={:?} (model install {:?})",
                                        pages.len(),
                                        ipc_done.duration_since(started),
                                        ui_start.elapsed(),
                                        diff_start.elapsed(),
                                    );
                                    TableShared::log_render(&ui, &label, started);
                                }
                                if installed {
                                    let ids = shared.ids.lock().map(|g| g.clone()).unwrap_or_default();
                                    let new_selected_index = previous_selection
                                        .and_then(|id| ids.iter().position(|candidate| *candidate == id))
                                        .map_or(-1, |index| index as i32);
                                    ui.set_selected(new_selected_index);
                                    let selected_task_id = usize::try_from(new_selected_index)
                                        .ok()
                                        .and_then(|idx| id_at(&ids, idx));
                                    let is_live = selected_task_id.is_some_and(|id| {
                                        let media_sec = shared
                                            .statuses
                                            .lock()
                                            .ok()
                                            .and_then(|map| map.get(&id).and_then(|entry| entry.1));
                                        is_live_task(&db_path_refresh, id, media_sec, &live_ids_refresh)
                                    });
                                    ui.set_pause_button_text(if is_live { "Stop and save".into() } else { "Pause".into() });
                                    ui.set_message(format!("{total} task(s)").into());
                                }
                            }
                            Err(error) => ui.set_message(error.into()),
                        }
                        if refresh_again.swap(false, Ordering::SeqCst) {
                            if let Some(again) = refresh_slot.get() {
                                again();
                            }
                        }
                    });
                });
            });
        })
    };
    let _ = refresh_slot.set(refresh.clone());
    let refresh_button = refresh.clone();
    ui.on_refresh(move || {
        refresh_button();
    });

    let weak_select = ui.as_weak();
    let ids_select = task_ids.clone();
    let db_path_select = paths.dir.join("tasks.sqlite3");
    let live_ids_select = live_task_ids.clone();
    ui.on_task_selected(move |index| {
        let Some(ui) = weak_select.upgrade() else { return; };
        let task_id = usize::try_from(index)
            .ok()
            .and_then(|idx| ids_select.lock().ok().and_then(|guard| id_at(&guard, idx)));
        let is_live = task_id.is_some_and(|id| {
            is_live_task(&db_path_select, id, None, &live_ids_select)
        });
        ui.set_pause_button_text(if is_live { "Stop and save".into() } else { "Pause".into() });
    });

    let ipc_add = ipc.clone();
    let weak = ui.as_weak();
    let refresh_add = refresh.clone();
    let gui_settings_add = gui_settings.clone();
    ui.on_add_task(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let url = ui.get_url().to_string();
        let output = ui.get_destination().to_string();
        if url.is_empty() || output.is_empty() {
            ui.set_message("Enter a URL and destination first".into());
            return;
        }
        if let Err(err) = dialog_helpers::validate_download_url(&url) {
            ui.set_message(err.into());
            return;
        }
        let connections = gui_settings_add
            .lock()
            .map(|g| g.default_connections)
            .unwrap_or(settings::DEFAULT_CONNECTIONS);
        let request_id = format!(
            "gui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let weak = weak.clone();
        let refresh = refresh_add.clone();
        ipc_add.execute(move |paths| {
            let result = request(
                paths,
                Command::Submit {
                    request_id,
                    url,
                    output,
                    headers: Vec::new(),
                    priority: 1,
                    connections,
                },
            );
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ipc_start = ipc.clone();
    let refresh_start = refresh.clone();
    ui.on_queue_start(move || {
        let weak = weak.clone();
        let refresh = refresh_start.clone();
        ipc_start.execute(move |paths| {
            let result = request(paths, Command::QueueStart);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ipc_stop = ipc.clone();
    let refresh_stop = refresh.clone();
    ui.on_queue_stop(move || {
        let weak = weak.clone();
        let refresh = refresh_stop.clone();
        ipc_stop.execute(move |paths| {
            let result = request(paths, Command::QueueStop);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ipc_start_one = ipc.clone();
    let refresh_start_one = refresh.clone();
    ui.on_queue_start_one(move || {
        let Some(ui) = weak.upgrade() else { return; };
        let queue_id = ui.get_queue_filter_id().as_str().parse::<i64>().unwrap_or(0);
        if queue_id == 0 {
            ui.set_message("Select a specific queue first".into());
            return;
        }
        let weak = weak.clone();
        let refresh = refresh_start_one.clone();
        ipc_start_one.execute(move |paths| {
            let result = request(paths, Command::QueueStartOne { queue_id });
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ipc_stop_one = ipc.clone();
    let refresh_stop_one = refresh.clone();
    ui.on_queue_stop_one(move || {
        let Some(ui) = weak.upgrade() else { return; };
        let queue_id = ui.get_queue_filter_id().as_str().parse::<i64>().unwrap_or(0);
        if queue_id == 0 {
            ui.set_message("Select a specific queue first".into());
            return;
        }
        let weak = weak.clone();
        let refresh = refresh_stop_one.clone();
        ipc_stop_one.execute(move |paths| {
            let result = request(paths, Command::QueueStopOne { queue_id });
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ids_move = task_ids.clone();
    let ipc_move = ipc.clone();
    let refresh_move = refresh.clone();
    ui.on_move_task(move |direction| {
        let Some(ui) = weak.upgrade() else { return; };
        let Some(task_id) = selected_task(&ui, &ids_move) else {
            return;
        };
        let target = match direction.as_str() {
            "up" => MoveTarget::Up,
            "down" => MoveTarget::Down,
            "top" => MoveTarget::Top,
            "bottom" => MoveTarget::Bottom,
            _ => return,
        };
        let weak = weak.clone();
        let refresh = refresh_move.clone();
        ipc_move.execute(move |paths| {
            let result = request(paths, Command::TaskMove { task_id, target });
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ipc_schedule = ipc.clone();
    let refresh_schedule = refresh.clone();
    ui.on_schedule_queue_start(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let date =
            NaiveDateTime::parse_from_str(ui.get_schedule_start().as_str(), "%Y-%m-%d %H:%M")
                .ok()
                .and_then(|date| Local.from_local_datetime(&date).single());
        let Some(date) = date else {
            ui.set_message("Enter an unambiguous local date and time: YYYY-MM-DD HH:MM".into());
            return;
        };
        let weak = weak.clone();
        let refresh = refresh_schedule.clone();
        ipc_schedule.execute(move |paths| {
            let result = request(
                paths,
                Command::QueueScheduleStart {
                    at_unix: date.timestamp(),
                },
            );
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ipc_stop_schedule = ipc.clone();
    let refresh_stop_schedule = refresh.clone();
    ui.on_schedule_queue_stop(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let date = NaiveDateTime::parse_from_str(ui.get_schedule_stop().as_str(), "%Y-%m-%d %H:%M")
            .ok()
            .and_then(|date| Local.from_local_datetime(&date).single());
        let Some(date) = date else {
            ui.set_message("Enter an unambiguous local stop time: YYYY-MM-DD HH:MM".into());
            return;
        };
        let weak = weak.clone();
        let refresh = refresh_stop_schedule.clone();
        ipc_stop_schedule.execute(move |paths| {
            let result = request(
                paths,
                Command::QueueScheduleStop {
                    at_unix: date.timestamp(),
                },
            );
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    for (pause, callback) in [
        (
            true,
            MainWindow::on_pause_task as fn(&MainWindow, Box<dyn Fn()>),
        ),
        (
            false,
            MainWindow::on_resume_task as fn(&MainWindow, Box<dyn Fn()>),
        ),
    ] {
        let weak = ui.as_weak();
        let ids = task_ids.clone();
        let sel_set = selected_task_ids.clone();
        let all_sel = table_shared.all.clone();
        let ipc = ipc.clone();
        let refresh = refresh.clone();
        callback(
            &ui,
            Box::new(move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let target = resolve_target(&ui, &ids, &sel_set, &all_sel);
                if target.count() == 0 {
                    ui.set_message("Select a task first".into());
                    return;
                }
                let weak = weak.clone();
                let refresh = refresh.clone();
                ipc.execute(move |paths| {
                    let result = match target {
                        Target::All(all) => bulk_message(
                            paths
                                .request(bulk_command(
                                    &all,
                                    if pause { xdm_ipc::BulkAction::Pause } else { xdm_ipc::BulkAction::Resume },
                                ))
                                .map_err(|e| e.to_string()),
                            if pause { "Paused" } else { "Resumed" },
                        ),
                        Target::Ids(targets) => {
                            let mut last = String::new();
                            for id in &targets {
                                let command = if pause {
                                    Command::Pause { task_id: *id }
                                } else {
                                    Command::Resume { task_id: *id }
                                };
                                last = request(paths, command);
                            }
                            if targets.len() > 1 {
                                format!("{} {} task(s)", if pause { "Paused" } else { "Resumed" }, targets.len())
                            } else {
                                last
                            }
                        }
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_message(result.into());
                        }
                        refresh();
                    });
                });
            }),
        );
    }

    let weak = ui.as_weak();
    let ids_priority = task_ids.clone();
    let sel_priority = selected_task_ids.clone();
    let all_priority = table_shared.all.clone();
    let ipc_priority = ipc.clone();
    let refresh_priority = refresh.clone();
    ui.on_set_priority(move |priority| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let target = match resolve_target(&ui, &ids_priority, &sel_priority, &all_priority) {
            Target::Ids(_) => {
                let Some(task_id) = usize::try_from(ui.get_selected())
                    .ok()
                    .and_then(|index| ids_priority.lock().ok().and_then(|guard| id_at(&guard, index)))
                else {
                    ui.set_message("Select a task first".into());
                    return;
                };
                Target::Ids(vec![task_id])
            }
            all => all,
        };
        let weak = weak.clone();
        let refresh = refresh_priority.clone();
        ipc_priority.execute(move |paths| {
            let result = match target {
                Target::All(all) => bulk_message(
                    paths
                        .request(bulk_command(&all, xdm_ipc::BulkAction::Priority { level: priority as i64 }))
                        .map_err(|e| e.to_string()),
                    "Changed priority of",
                ),
                Target::Ids(ids) => request(
                    paths,
                    Command::SetPriority {
                        task_id: ids[0],
                        priority: priority as i64,
                    },
                ),
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    let weak = ui.as_weak();
    let ids_rate = task_ids.clone();
    let ipc_rate = ipc.clone();
    let refresh_rate = refresh.clone();
    ui.on_set_task_rate(move |limited| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(task_id) = usize::try_from(ui.get_selected())
            .ok()
            .and_then(|index| ids_rate.lock().ok().and_then(|guard| id_at(&guard, index)))
        else {
            ui.set_message("Select a task first".into());
            return;
        };
        let rate = if limited {
            let Some(rate) = ui
                .get_rate_kib()
                .as_str()
                .parse::<u64>()
                .ok()
                .filter(|rate| *rate > 0)
                .and_then(|rate| rate.checked_mul(1024))
            else {
                ui.set_message("Enter a positive speed in KiB/s".into());
                return;
            };
            Some(rate)
        } else {
            None
        };
        let weak = weak.clone();
        let refresh = refresh_rate.clone();
        ipc_rate.execute(move |paths| {
            let result = request(
                paths,
                Command::SetTaskRate {
                    task_id,
                    bytes_per_second: rate,
                },
            );
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_message(result.into());
                }
                refresh();
            });
        });
    });

    // --- selection, sorting, keyboard -------------------------------------
    let anchor = Rc::new(std::cell::Cell::new(-1i32));
    {
        let ids = task_ids.clone();
        let set = selected_task_ids.clone();
        let anchor = anchor.clone();
        let all_click = table_shared.all.clone();
        let weak = ui.as_weak();
        ui.on_task_clicked(move |index, ctrl, shift| {
            let Some(ui) = weak.upgrade() else { return; };
            if let Ok(mut all) = all_click.lock() {
                *all = None;
            }
            let ids = ids.lock().map(|g| g.clone()).unwrap_or_default();
            let Some(id) = usize::try_from(index).ok().and_then(|i| id_at(&ids, i)) else { return; };
            let Ok(mut set) = set.lock() else { return; };
            let prev = anchor.get();
            if shift && prev >= 0 {
                let (lo, hi) = (prev.min(index) as usize, prev.max(index) as usize);
                set.clear();
                for i in lo..=hi.min(ids.len().saturating_sub(1)) {
                    if let Some(id) = id_at(&ids, i) {
                        set.insert(id as usize);
                    }
                }
            } else if ctrl {
                if set.is_empty() {
                    if let Some(pid) = usize::try_from(prev).ok().and_then(|i| id_at(&ids, i)) {
                        set.insert(pid as usize);
                    }
                }
                if !set.remove(&(id as usize)) {
                    set.insert(id as usize);
                }
                anchor.set(index);
            } else {
                set.clear();
                set.insert(id as usize);
                anchor.set(index);
            }
            sync_selection(&ui, &set);
        });
    }
    {
        let sort_state = sort_state.clone();
        let weak = ui.as_weak();
        let refresh = refresh.clone();
        ui.on_sort_by_column(move |col| {
            let Some(ui) = weak.upgrade() else { return; };
            if let Ok(mut state) = sort_state.lock() {
                if state.0 == col.as_str() {
                    state.1 = !state.1;
                } else {
                    *state = (col.to_string(), true);
                }
                ui.set_sort_column(state.0.clone().into());
                ui.set_sort_asc(state.1);
            }
            refresh();
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_key_delete(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_remove_task();
            }
        });
        let weak = ui.as_weak();
        ui.on_key_enter(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_task_details();
            }
        });
        let weak = ui.as_weak();
        let ids = task_ids.clone();
        let set = selected_task_ids.clone();
        let anchor_all = anchor.clone();
        let all_key = table_shared.all.clone();
        ui.on_key_select_all(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let ids = ids.lock().map(|g| g.clone()).unwrap_or_default();
            if ids.is_empty() {
                return;
            }
            // Every task matching the current filter, not just the loaded pages;
            // the daemon re-checks the count the user saw.
            if let Ok(mut all) = all_key.lock() {
                *all = Some(AllMatching { filter: current_filter(&ui), total: ids.len() as u64 });
            }
            if let Ok(mut set) = set.lock() {
                *set = ids.iter().filter(|id| **id > 0).map(|id| *id as usize).collect();
                sync_selection(&ui, &set);
            }
            ui.set_message(format!("All {} task(s) matching the filter selected", ids.len()).into());
            if ui.get_selected() < 0 {
                ui.set_selected(0);
                anchor_all.set(0);
            }
        });
        for delta in [-1i32, 1] {
            let weak = ui.as_weak();
            let ids = task_ids.clone();
            let set = selected_task_ids.clone();
            let anchor = anchor.clone();
            let all_nav = table_shared.all.clone();
            let handler = move || {
                let Some(ui) = weak.upgrade() else { return; };
                if let Ok(mut all) = all_nav.lock() {
                    *all = None;
                }
                let ids = ids.lock().map(|g| g.clone()).unwrap_or_default();
                if ids.is_empty() {
                    return;
                }
                let cur = ui.get_selected();
                let next = if cur < 0 {
                    if delta > 0 { 0 } else { ids.len() as i32 - 1 }
                } else {
                    (cur + delta).clamp(0, ids.len() as i32 - 1)
                };
                ui.set_selected(next);
                anchor.set(next);
                if let Ok(mut set) = set.lock() {
                    set.clear();
                    if let Some(id) = id_at(&ids, next as usize) {
                        set.insert(id as usize);
                    }
                    sync_selection(&ui, &set);
                }
                ui.invoke_task_selected(next);
            };
            if delta < 0 {
                ui.on_key_up(handler);
            } else {
                ui.on_key_down(handler);
            }
        }
    }

    // --- open / open folder / move-rename ---------------------------------
    for folder in [false, true] {
        let weak = ui.as_weak();
        let ipc_open = ipc.clone();
        let ids_open = task_ids.clone();
        let handler = move || {
            let Some(ui) = weak.upgrade() else { return; };
            let Some(task_id) = selected_task(&ui, &ids_open) else { return; };
            let weak = weak.clone();
            ipc_then(&ipc_open, move |paths| fetch_task(paths, task_id), move |result| {
                let Some(ui) = weak.upgrade() else { return; };
                match result {
                    Ok(task) => {
                        if task.output.is_empty() {
                            ui.set_message("Task has no output path".into());
                        } else if folder {
                            task_table::open_folder(&task.output);
                            ui.set_message(format!("Opening folder of {}", task.output).into());
                        } else if !task_table::is_complete(&task.status) {
                            ui.set_message("Only completed downloads can be opened".into());
                        } else {
                            task_table::open_file(&task.output);
                            ui.set_message(format!("Opening {}", task.output).into());
                        }
                    }
                    Err(message) => ui.set_message(message.into()),
                }
            });
        };
        if folder {
            ui.on_open_task_folder(handler);
        } else {
            ui.on_open_task(handler);
        }
    }

    let move_window = Rc::new(RefCell::new(None::<MoveRenameWindow>));
    {
        let weak = ui.as_weak();
        let ids_move = task_ids.clone();
        let ipc_move = ipc.clone();
        let refresh_move = refresh.clone();
        ui.on_move_rename_task(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let Some(task_id) = selected_task(&ui, &ids_move) else { return; };
            let weak_outer = weak.clone();
            let refresh_move = refresh_move.clone();
            let ipc_move = ipc_move.clone();
            let move_window = move_window.clone();
            ipc_then(&ipc_move.clone(), move |paths| fetch_task(paths, task_id), move |result| {
            let Some(ui) = weak_outer.upgrade() else { return; };
            let task = match result {
                Ok(task) => task,
                Err(message) => {
                    ui.set_message(message.into());
                    return;
                }
            };
            if !task_table::is_complete(&task.status) {
                ui.set_message("Move / Rename is only available for completed downloads".into());
                return;
            }
            let dialog = match MoveRenameWindow::new() {
                Ok(d) => d,
                Err(error) => {
                    ui.set_message(error.to_string().into());
                    return;
                }
            };
            install_tr!(dialog);
            dialog.set_current_path(task.output.clone().into());
            dialog.set_target_path(task.output.clone().into());
            let weak_dialog = dialog.as_weak();
            let weak_ui = ui.as_weak();
            let refresh = refresh_move.clone();
            let source = task.output.clone();
            let ipc_move_file = ipc_move.clone();
            dialog.on_submit(move || {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                let target = dialog.get_target_path().to_string();
                if target.trim().is_empty() || target == source {
                    dialog.set_message("Enter a different destination path".into());
                    return;
                }
                // The daemon moves the file and updates the task record; its error
                // text (for example "destination exists") is shown verbatim.
                dialog.set_message("Moving…".into());
                let new_path = target.trim().to_owned();
                let weak_dialog = weak_dialog.clone();
                let weak_ui = weak_ui.clone();
                let refresh = refresh.clone();
                ipc_move_file.execute(move |paths| {
                    let result = paths.request(Command::TaskMoveFile { task_id, new_path: new_path.clone() });
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(dialog) = weak_dialog.upgrade() else { return; };
                        match result {
                            Ok(reply) if reply.ok => {
                                let _ = dialog.hide();
                                if let Some(ui) = weak_ui.upgrade() {
                                    ui.set_message(format!("Moved to {new_path}").into());
                                }
                                refresh();
                            }
                            Ok(reply) => dialog.set_message(reply.error.unwrap_or_else(|| "Move rejected".into()).into()),
                            Err(error) => dialog.set_message(format!("Daemon unavailable: {error}").into()),
                        }
                    });
                });
            });
            let weak_dialog = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(d) = weak_dialog.upgrade() {
                    let _ = d.hide();
                }
            });
            if let Some(prev) = move_window.borrow_mut().take() {
                let _ = prev.hide();
            }
            if let Err(error) = dialog.show() {
                ui.set_message(error.to_string().into());
            }
            *move_window.borrow_mut() = Some(dialog);
            });
        });
    }

    // --- restart a task parked in restart_required ------------------------
    {
        let weak = ui.as_weak();
        let ipc_restart = ipc.clone();
        let ids = task_ids.clone();
        let sel = selected_task_ids.clone();
        let all_restart = table_shared.all.clone();
        let refresh_restart = refresh.clone();
        ui.on_restart_task(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let target = resolve_target(&ui, &ids, &sel, &all_restart);
            if target.count() == 0 {
                ui.set_message("Select a task first".into());
                return;
            }
            let weak = weak.clone();
            let refresh = refresh_restart.clone();
            ipc_restart.execute(move |paths| {
                let targets = match target {
                    Target::Ids(ids) => ids,
                    Target::All(all) => {
                        let message = bulk_message(
                            paths.request(bulk_command(&all, xdm_ipc::BulkAction::Restart)).map_err(|e| e.to_string()),
                            "Restarted",
                        );
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() {
                                refresh();
                                ui.set_message(message.into());
                            }
                        });
                        return;
                    }
                };
                let mut message = format!("Restarted {} task(s) from byte zero", targets.len());
                for task_id in &targets {
                    match paths.request(Command::TaskRestart { task_id: *task_id }) {
                        Ok(reply) if reply.ok => {}
                        Ok(reply) => {
                            message = format!("Task #{task_id}: {}", reply.error.unwrap_or_else(|| "Restart rejected".into()));
                            break;
                        }
                        Err(error) => {
                            message = format!("Daemon unavailable: {error}");
                            break;
                        }
                    }
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        refresh();
                        ui.set_message(message.into());
                    }
                });
            });
        });
    }

    // --- clean up unowned partial files -----------------------------------
    let partials_window = Rc::new(RefCell::new(None::<PartialsWindow>));
    {
        let weak_main = ui.as_weak();
        let ipc_partials = ipc.clone();
        let default_dir = paths.dir.join("downloads").to_string_lossy().into_owned();
        ui.on_open_partials(move || {
            let Some(ui) = weak_main.upgrade() else { return; };
            let dialog = match PartialsWindow::new() {
                Ok(dialog) => dialog,
                Err(error) => {
                    ui.set_message(error.to_string().into());
                    return;
                }
            };
            dialog.set_directory(default_dir.clone().into());
            let entries = Arc::new(Mutex::new(Vec::<(xdm_ipc::PartialEntry, bool)>::new()));
            let scanned_dir = Arc::new(Mutex::new(String::new()));
            let render: Arc<dyn Fn(&PartialsWindow) + Send + Sync> = {
                let entries = entries.clone();
                Arc::new(move |dialog: &PartialsWindow| {
                    let items: Vec<PartialItem> = entries
                        .lock()
                        .map(|guard| {
                            guard
                                .iter()
                                .map(|(entry, checked)| PartialItem {
                                    label: maintenance::partial_label(entry).into(),
                                    checked: *checked,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    dialog.set_items(ModelRc::new(VecModel::from(items)));
                })
            };
            let scan: Arc<dyn Fn() + Send + Sync> = {
                let weak = dialog.as_weak();
                let ipc = ipc_partials.clone();
                let entries = entries.clone();
                let scanned_dir = scanned_dir.clone();
                let render = render.clone();
                Arc::new(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    let directory = dialog.get_directory().trim().to_owned();
                    dialog.set_message("Scanning…".into());
                    dialog.set_confirming(false);
                    let (weak, entries, scanned_dir, render) = (weak.clone(), entries.clone(), scanned_dir.clone(), render.clone());
                    ipc.execute(move |paths| {
                        let result = paths.request(Command::PartialScan { directory: directory.clone() });
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(dialog) = weak.upgrade() else { return; };
                            match result {
                                Ok(reply) if reply.ok => {
                                    let count = reply.partials.len();
                                    let bytes: u64 = reply.partials.iter().map(maintenance::partial_bytes).sum();
                                    if let Ok(mut guard) = entries.lock() {
                                        *guard = reply.partials.into_iter().map(|entry| (entry, false)).collect();
                                    }
                                    if let Ok(mut guard) = scanned_dir.lock() {
                                        *guard = directory;
                                    }
                                    render(&dialog);
                                    dialog.set_notice(if reply.truncated {
                                        format!("Only the first {count} entries are listed (scan limit reached). Delete these and scan again for more.").into()
                                    } else {
                                        "".into()
                                    });
                                    dialog.set_message(format!("{count} unowned partial entr{} ({})", if count == 1 { "y" } else { "ies" }, maintenance::human_bytes(bytes)).into());
                                }
                                Ok(reply) => dialog.set_message(reply.error.unwrap_or_else(|| "Scan rejected".into()).into()),
                                Err(error) => dialog.set_message(format!("Daemon unavailable: {error}").into()),
                            }
                        });
                    });
                })
            };
            let scan_button = scan.clone();
            dialog.on_scan(move || scan_button());
            {
                let entries = entries.clone();
                let render = render.clone();
                let weak = dialog.as_weak();
                dialog.on_toggle(move |index| {
                    let Some(dialog) = weak.upgrade() else { return; };
                    if let Ok(mut guard) = entries.lock() {
                        if let Some(item) = usize::try_from(index).ok().and_then(|i| guard.get_mut(i)) {
                            item.1 = !item.1;
                        }
                    }
                    dialog.set_confirming(false);
                    render(&dialog);
                });
            }
            {
                let entries = entries.clone();
                let render = render.clone();
                let weak = dialog.as_weak();
                dialog.on_select_all(move |all| {
                    let Some(dialog) = weak.upgrade() else { return; };
                    if let Ok(mut guard) = entries.lock() {
                        for item in guard.iter_mut() {
                            item.1 = all;
                        }
                    }
                    dialog.set_confirming(false);
                    render(&dialog);
                });
            }
            {
                let entries = entries.clone();
                let scanned_dir = scanned_dir.clone();
                let weak = dialog.as_weak();
                dialog.on_delete_selected(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    let (count, bytes) = entries
                        .lock()
                        .map(|guard| {
                            guard
                                .iter()
                                .filter(|(_, checked)| *checked)
                                .fold((0usize, 0u64), |(c, b), (entry, _)| (c + 1, b + maintenance::partial_bytes(entry)))
                        })
                        .unwrap_or((0, 0));
                    if count == 0 {
                        dialog.set_message("Select at least one entry first".into());
                        return;
                    }
                    let directory = scanned_dir.lock().map(|g| g.clone()).unwrap_or_default();
                    dialog.set_confirm_text(format!("Permanently delete {count} partial entr{} ({}) from {directory}? Files of active tasks are never touched.", if count == 1 { "y" } else { "ies" }, maintenance::human_bytes(bytes)).into());
                    dialog.set_confirming(true);
                });
            }
            {
                let weak = dialog.as_weak();
                dialog.on_cancel_confirm(move || {
                    if let Some(dialog) = weak.upgrade() {
                        dialog.set_confirming(false);
                    }
                });
            }
            {
                let entries = entries.clone();
                let scanned_dir = scanned_dir.clone();
                let ipc = ipc_partials.clone();
                let scan = scan.clone();
                let weak = dialog.as_weak();
                dialog.on_confirm_delete(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    dialog.set_confirming(false);
                    let paths_to_delete: Vec<String> = entries
                        .lock()
                        .map(|guard| {
                            guard
                                .iter()
                                .filter(|(_, checked)| *checked)
                                .flat_map(|(entry, _)| maintenance::partial_paths(entry))
                                .collect()
                        })
                        .unwrap_or_default();
                    let directory = scanned_dir.lock().map(|g| g.clone()).unwrap_or_default();
                    let weak = weak.clone();
                    let scan = scan.clone();
                    ipc.execute(move |paths| {
                        let result = paths.request(Command::PartialDelete { directory, paths: paths_to_delete });
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(dialog) = weak.upgrade() else { return; };
                            match result {
                                Ok(reply) if reply.ok => {
                                    scan();
                                    // The rescan sets its own text; keep the outcome visible after it.
                                    dialog.set_notice(format!("Deleted {} file(s).", reply.total).into());
                                }
                                Ok(reply) => dialog.set_message(reply.error.unwrap_or_else(|| "Delete rejected".into()).into()),
                                Err(error) => dialog.set_message(format!("Daemon unavailable: {error}").into()),
                            }
                        });
                    });
                });
            }
            {
                let weak = dialog.as_weak();
                dialog.on_dismiss(move || {
                    if let Some(dialog) = weak.upgrade() {
                        let _ = dialog.hide();
                    }
                });
            }
            if let Some(previous) = partials_window.borrow_mut().take() {
                let _ = previous.hide();
            }
            if let Err(error) = dialog.show() {
                ui.set_message(error.to_string().into());
            }
            *partials_window.borrow_mut() = Some(dialog);
        });
    }

    // --- bandwidth limits, host budgets and quota --------------------------
    let bandwidth_window = Rc::new(RefCell::new(None::<BandwidthWindow>));
    let open_bandwidth: Rc<dyn Fn()> = {
        let weak_main = ui.as_weak();
        let ipc_bw = ipc.clone();
        Rc::new(move || {
            let Some(ui) = weak_main.upgrade() else { return; };
            let dialog = match BandwidthWindow::new() {
                Ok(dialog) => dialog,
                Err(error) => {
                    ui.set_message(error.to_string().into());
                    return;
                }
            };
            let report = Arc::new(Mutex::new(xdm_ipc::BandwidthReport::default()));
            // Runs an optional change, then reads the status back so the dialog
            // always shows what the daemon really holds.
            let run: Arc<dyn Fn(Option<Command>, &str) + Send + Sync> = {
                let weak = dialog.as_weak();
                let ipc = ipc_bw.clone();
                let report = report.clone();
                Arc::new(move |change: Option<Command>, done: &str| {
                    let weak = weak.clone();
                    let report = report.clone();
                    let done = done.to_owned();
                    ipc.execute(move |paths| {
                        let change_error = match change {
                            Some(command) => match paths.request(command) {
                                Ok(reply) if reply.ok => None,
                                Ok(reply) => Some(reply.error.unwrap_or_else(|| "Rejected by the daemon".into())),
                                Err(error) => Some(format!("Daemon unavailable: {error}")),
                            },
                            None => None,
                        };
                        let status = paths.request(Command::BandwidthStatus);
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(dialog) = weak.upgrade() else { return; };
                            match status {
                                Ok(reply) if reply.ok => {
                                    if let Some(current) = reply.bandwidth {
                                        let now = Local::now().timestamp();
                                        dialog.set_status_text(maintenance::format_bandwidth_status(&current, now).into());
                                        dialog.set_host_limits(ModelRc::new(VecModel::from(
                                            maintenance::format_host_limits(&current.host_limits).into_iter().map(slint::SharedString::from).collect::<Vec<_>>(),
                                        )));
                                        dialog.set_host_conns(ModelRc::new(VecModel::from(
                                            maintenance::format_host_budgets(&current.host_connections).into_iter().map(slint::SharedString::from).collect::<Vec<_>>(),
                                        )));
                                        dialog.set_global_limit(current.global_kib.map(|kib| kib.to_string()).unwrap_or_default().into());
                                        dialog.set_quota_mib(if current.quota_mib == 0 { String::new() } else { current.quota_mib.to_string() }.into());
                                        if current.quota_mib > 0 {
                                            dialog.set_quota_hours((current.quota_window_seconds / 3600).max(1).to_string().into());
                                        }
                                        if let Ok(mut guard) = report.lock() {
                                            *guard = current;
                                        }
                                    }
                                    dialog.set_message(change_error.unwrap_or(done).into());
                                }
                                Ok(reply) => dialog.set_message(reply.error.unwrap_or_else(|| "Status unavailable".into()).into()),
                                Err(error) => dialog.set_message(format!("Daemon unavailable: {error}").into()),
                            }
                        });
                    });
                })
            };
            {
                let weak = dialog.as_weak();
                let run = run.clone();
                dialog.on_apply_global(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    match maintenance::parse_optional_number(dialog.get_global_limit().as_str(), "Global limit") {
                        Ok(kib) => run(Some(Command::SetGlobalLimit { kib }), "Global limit applied"),
                        Err(error) => dialog.set_message(error.into()),
                    }
                });
            }
            {
                let run = run.clone();
                dialog.on_clear_global(move || run(Some(Command::SetGlobalLimit { kib: None }), "Global limit removed"));
            }
            {
                let weak = dialog.as_weak();
                let run = run.clone();
                dialog.on_set_host_limit(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    let pattern = dialog.get_host_pattern().trim().to_owned();
                    match maintenance::parse_optional_number(dialog.get_host_kib().as_str(), "Host limit") {
                        Ok(Some(kib)) if !pattern.is_empty() => run(Some(Command::SetHostLimit { pattern, kib: Some(kib) }), "Host limit saved"),
                        Ok(_) => dialog.set_message("Enter a host pattern and a KiB/s value (0 = unlimited)".into()),
                        Err(error) => dialog.set_message(error.into()),
                    }
                });
            }
            {
                let weak = dialog.as_weak();
                let run = run.clone();
                dialog.on_remove_host_limit(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    let pattern = dialog.get_host_pattern().trim().to_owned();
                    if pattern.is_empty() {
                        dialog.set_message("Pick or type the host pattern to remove".into());
                    } else {
                        run(Some(Command::SetHostLimit { pattern, kib: None }), "Host limit rule removed");
                    }
                });
            }
            {
                let weak = dialog.as_weak();
                let report = report.clone();
                dialog.on_pick_host_limit(move |index| {
                    let Some(dialog) = weak.upgrade() else { return; };
                    if let Some((pattern, kib)) = report.lock().ok().and_then(|g| usize::try_from(index).ok().and_then(|i| g.host_limits.get(i).cloned())) {
                        dialog.set_host_pattern(pattern.into());
                        dialog.set_host_kib(kib.to_string().into());
                    }
                });
            }
            {
                let weak = dialog.as_weak();
                let run = run.clone();
                dialog.on_set_host_conns(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    let pattern = dialog.get_conn_pattern().trim().to_owned();
                    match maintenance::parse_optional_number(dialog.get_conn_count().as_str(), "Connection budget") {
                        Ok(Some(count)) if !pattern.is_empty() => run(Some(Command::SetHostConnections { pattern, count: Some(count) }), "Connection budget saved"),
                        Ok(_) => dialog.set_message("Enter a host pattern and a connection count".into()),
                        Err(error) => dialog.set_message(error.into()),
                    }
                });
            }
            {
                let weak = dialog.as_weak();
                let run = run.clone();
                dialog.on_remove_host_conns(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    let pattern = dialog.get_conn_pattern().trim().to_owned();
                    if pattern.is_empty() {
                        dialog.set_message("Pick or type the host pattern to remove".into());
                    } else {
                        run(Some(Command::SetHostConnections { pattern, count: None }), "Connection budget rule removed");
                    }
                });
            }
            {
                let weak = dialog.as_weak();
                let report = report.clone();
                dialog.on_pick_host_conns(move |index| {
                    let Some(dialog) = weak.upgrade() else { return; };
                    if let Some((pattern, count)) = report.lock().ok().and_then(|g| usize::try_from(index).ok().and_then(|i| g.host_connections.get(i).cloned())) {
                        dialog.set_conn_pattern(pattern.into());
                        dialog.set_conn_count(count.to_string().into());
                    }
                });
            }
            {
                let weak = dialog.as_weak();
                let run = run.clone();
                dialog.on_apply_quota(move || {
                    let Some(dialog) = weak.upgrade() else { return; };
                    match maintenance::parse_quota(dialog.get_quota_mib().as_str(), dialog.get_quota_hours().as_str()) {
                        Ok((mib, window_seconds)) => run(Some(Command::SetQuota { mib, window_seconds }), if mib == 0 { "Quota off" } else { "Quota applied" }),
                        Err(error) => dialog.set_message(error.into()),
                    }
                });
            }
            {
                let run = run.clone();
                dialog.on_disable_quota(move || run(Some(Command::SetQuota { mib: 0, window_seconds: 3600 }), "Quota off"));
            }
            {
                let run = run.clone();
                dialog.on_refresh(move || run(None, "Status refreshed"));
            }
            {
                let weak = dialog.as_weak();
                dialog.on_dismiss(move || {
                    if let Some(dialog) = weak.upgrade() {
                        let _ = dialog.hide();
                    }
                });
            }
            run(None, "Loaded from the daemon");
            if let Some(previous) = bandwidth_window.borrow_mut().take() {
                let _ = previous.hide();
            }
            if let Err(error) = dialog.show() {
                ui.set_message(error.to_string().into());
            }
            *bandwidth_window.borrow_mut() = Some(dialog);
        })
    };
    {
        let open = open_bandwidth.clone();
        ui.on_open_bandwidth(move || open());
    }

    // --- column visibility -------------------------------------------------
    let columns_window = Rc::new(RefCell::new(None::<ColumnsWindow>));
    {
        let weak = ui.as_weak();
        let settings_cols = gui_settings.clone();
        let dir_cols = paths.dir.clone();
        ui.on_open_columns(move || {
            let Some(ui) = weak.upgrade() else { return; };
            let dialog = match ColumnsWindow::new() {
                Ok(d) => d,
                Err(error) => {
                    ui.set_message(error.to_string().into());
                    return;
                }
            };
            install_tr!(dialog);
            dialog.set_col_filename(ui.get_col_filename());
            dialog.set_col_size(ui.get_col_size());
            dialog.set_col_status(ui.get_col_status());
            dialog.set_col_time_left(ui.get_col_time_left());
            dialog.set_col_transfer_rate(ui.get_col_transfer_rate());
            dialog.set_col_last_try_date(ui.get_col_last_try_date());
            dialog.set_col_description(ui.get_col_description());
            dialog.set_col_category(ui.get_col_category());
            dialog.set_col_queue(ui.get_col_queue());
            let weak_dialog = dialog.as_weak();
            let weak_ui = ui.as_weak();
            let settings = settings_cols.clone();
            let dir = dir_cols.clone();
            dialog.on_apply_columns(move || {
                let (Some(d), Some(ui)) = (weak_dialog.upgrade(), weak_ui.upgrade()) else { return; };
                ui.set_col_filename(d.get_col_filename());
                ui.set_col_size(d.get_col_size());
                ui.set_col_status(d.get_col_status());
                ui.set_col_time_left(d.get_col_time_left());
                ui.set_col_transfer_rate(d.get_col_transfer_rate());
                ui.set_col_last_try_date(d.get_col_last_try_date());
                ui.set_col_description(d.get_col_description());
                ui.set_col_category(d.get_col_category());
                ui.set_col_queue(d.get_col_queue());
                let mut new_settings = settings.lock().map(|s| s.clone()).unwrap_or_default();
                new_settings.columns = settings::ColumnSettings {
                    filename: d.get_col_filename(),
                    size: d.get_col_size(),
                    status: d.get_col_status(),
                    time_left: d.get_col_time_left(),
                    transfer_rate: d.get_col_transfer_rate(),
                    last_try_date: d.get_col_last_try_date(),
                    description: d.get_col_description(),
                    category: d.get_col_category(),
                    queue: d.get_col_queue(),
                };
                match settings::save_settings(&dir, &new_settings) {
                    Ok(()) => {
                        if let Ok(mut guard) = settings.lock() {
                            *guard = new_settings;
                        }
                        ui.set_message("Columns saved".into());
                        let _ = d.hide();
                    }
                    Err(error) => ui.set_message(format!("Failed to save columns: {error}").into()),
                }
            });
            let weak_dialog = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(d) = weak_dialog.upgrade() {
                    let _ = d.hide();
                }
            });
            if let Some(prev) = columns_window.borrow_mut().take() {
                let _ = prev.hide();
            }
            if let Err(error) = dialog.show() {
                ui.set_message(error.to_string().into());
            }
            *columns_window.borrow_mut() = Some(dialog);
        });
    }

    let details_window = Rc::new(RefCell::new(None::<DetailsWindow>));
    let weak = ui.as_weak();
    let ipc_details = ipc.clone();
    let ids_details = task_ids.clone();
    ui.on_task_details(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(task_id) = selected_task(&ui, &ids_details) else {
            return;
        };
        let weak = weak.clone();
        let details_window = details_window.clone();
        ipc_then(&ipc_details, move |paths| fetch_task(paths, task_id), move |result| {
            let Some(ui) = weak.upgrade() else { return; };
            let task = match result {
                Ok(task) => task,
                Err(message) => {
                    ui.set_message(message.into());
                    return;
                }
            };
            match DetailsWindow::new() {
                Ok(dialog) => {
                    dialog.set_details(task_details(&task).into());
                    let weak = dialog.as_weak();
                    dialog.on_dismiss(move || {
                        if let Some(dialog) = weak.upgrade() {
                            let _ = dialog.hide();
                        }
                    });
                    if let Some(previous) = details_window.borrow_mut().take() {
                        let _ = previous.hide();
                    }
                    if let Err(error) = dialog.show() {
                        ui.set_message(error.to_string().into());
                    }
                    *details_window.borrow_mut() = Some(dialog);
                }
                Err(error) => ui.set_message(error.to_string().into()),
            }
        });
    });

    let redownload_window = Rc::new(RefCell::new(None::<RedownloadWindow>));
    let weak = ui.as_weak();
    let ipc_redownload = ipc.clone();
    let ids_redownload = task_ids.clone();
    let refresh_redownload = refresh.clone();
    ui.on_redownload_task(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(task_id) = selected_task(&ui, &ids_redownload) else {
            return;
        };
        let weak_outer = weak.clone();
        let ipc_redownload = ipc_redownload.clone();
        let refresh_redownload = refresh_redownload.clone();
        let redownload_window = redownload_window.clone();
        ipc_then(&ipc_redownload.clone(), move |paths| fetch_task(paths, task_id), move |result| {
        let Some(ui) = weak_outer.upgrade() else { return; };
        let task = match result {
            Ok(task) => task,
            Err(message) => {
                ui.set_message(message.into());
                return;
            }
        };
        let dialog = match RedownloadWindow::new() {
            Ok(dialog) => dialog,
            Err(error) => {
                ui.set_message(error.to_string().into());
                return;
            }
        };
        let output = std::path::Path::new(&task.output);
        let stem = output.file_stem().unwrap_or_default().to_string_lossy();
        let extension = output
            .extension()
            .map_or_else(String::new, |ext| format!(".{}", ext.to_string_lossy()));
        dialog.set_output(
            output
                .with_file_name(format!("{stem}-again{extension}"))
                .to_string_lossy()
                .into_owned()
                .into(),
        );
        let ipc_submit = ipc_redownload.clone();
        let refresh = refresh_redownload.clone();
        let weak_dialog = dialog.as_weak();
        let weak_ui = ui.as_weak();
        let request_id = format!(
            "gui-redownload-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        dialog.on_submit(move || {
            let Some(dialog) = weak_dialog.upgrade() else {
                return;
            };
            dialog.set_message("Queueing…".into());
            let command = Command::Redownload {
                task_id,
                request_id: request_id.clone(),
                output: dialog.get_output().to_string(),
            };
            let weak_dialog = weak_dialog.clone();
            let weak_ui = weak_ui.clone();
            let refresh = refresh.clone();
            ipc_then(&ipc_submit, move |paths| paths.request(command).map_err(|e| e.to_string()), move |result| {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                match result {
                    Ok(reply) if reply.ok => {
                        let _ = dialog.hide();
                        refresh();
                        if let Some(ui) = weak_ui.upgrade() {
                            ui.set_message(
                                format!("Redownload queued as task #{}", reply.task_id.unwrap_or_default())
                                    .into(),
                            );
                        }
                    }
                    Ok(reply) => dialog.set_message(
                        reply.error.unwrap_or_else(|| "Redownload rejected".into()).into(),
                    ),
                    Err(error) => dialog.set_message(error.into()),
                }
            });
        });
        let weak_dialog = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
        });
        if let Some(previous) = redownload_window.borrow_mut().take() {
            let _ = previous.hide();
        }
        if let Err(error) = dialog.show() {
            ui.set_message(error.to_string().into());
        }
        *redownload_window.borrow_mut() = Some(dialog);
        });
    });

    let remove_window = Rc::new(RefCell::new(None::<RemoveWindow>));
    let weak = ui.as_weak();
    let ipc_remove = ipc.clone();
    let ids_remove = task_ids.clone();
    let sel_remove = selected_task_ids.clone();
    let shared_remove = table_shared.clone();
    let refresh_remove = refresh.clone();
    let settings_remove = gui_settings.clone();
    ui.on_remove_task(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let target = resolve_target(&ui, &ids_remove, &sel_remove, &shared_remove.all);
        let count = target.count();
        if count == 0 {
            ui.set_message("Select a task first".into());
            return;
        }
        // Unfinished tasks whose own .part/.part.meta the user may also delete.
        let unfinished: std::collections::HashSet<i64> = match &target {
            Target::Ids(ids) => ids
                .iter()
                .copied()
                .filter(|id| shared_remove.status_of(*id).is_some_and(|status| !task_table::is_complete(&status)))
                .collect(),
            Target::All(_) => std::collections::HashSet::new(),
        };
        let partial_visible = matches!(target, Target::All(_)) || !unfinished.is_empty();
        // Runs the removal off the UI thread; `dialog` is the confirmation to close.
        let start = {
            let ipc = ipc_remove.clone();
            let sel = sel_remove.clone();
            let all_slot = shared_remove.all.clone();
            let refresh = refresh_remove.clone();
            let weak_ui = ui.as_weak();
            let target = target.clone();
            move |partial: std::collections::HashSet<i64>, partial_all: bool, dialog: Option<slint::Weak<RemoveWindow>>| {
                let job_target = target.clone();
                let (sel, all_slot, refresh, weak_ui) = (sel.clone(), all_slot.clone(), refresh.clone(), weak_ui.clone());
                ipc_then(
                    &ipc,
                    move |paths| run_remove(paths, &job_target, &partial, partial_all),
                    move |result| {
                        match result {
                            Ok(message) => {
                                if let Some(dialog) = dialog.as_ref().and_then(|d| d.upgrade()) {
                                    let _ = dialog.hide();
                                }
                                if let Ok(mut set) = sel.lock() {
                                    set.clear();
                                }
                                if let Ok(mut all) = all_slot.lock() {
                                    *all = None;
                                }
                                if let Some(ui) = weak_ui.upgrade() {
                                    ui.set_selected(-1);
                                    ui.set_message(message.into());
                                }
                                refresh();
                            }
                            Err(error) => {
                                if let Some(dialog) = dialog.as_ref().and_then(|d| d.upgrade()) {
                                    dialog.set_message(error.into());
                                } else if let Some(ui) = weak_ui.upgrade() {
                                    ui.set_message(error.into());
                                }
                                refresh();
                            }
                        }
                    },
                );
            }
        };
        let confirm = settings_remove.lock().map(|s| s.confirm_remove).unwrap_or(true);
        if !confirm {
            start(std::collections::HashSet::new(), false, None);
            return;
        }
        let dialog = match RemoveWindow::new() {
            Ok(dialog) => dialog,
            Err(error) => {
                ui.set_message(error.to_string().into());
                return;
            }
        };
        dialog.set_message(
            match &target {
                Target::Ids(ids) if ids.len() == 1 => format!("Remove task #{} from the download list?", ids[0]),
                _ => format!("Remove {count} tasks from the download list?"),
            }
            .into(),
        );
        dialog.set_partial_visible(partial_visible);
        let weak_dialog = dialog.as_weak();
        let is_all = matches!(target, Target::All(_));
        dialog.on_confirm(move || {
            let Some(dialog) = weak_dialog.upgrade() else {
                return;
            };
            let delete = dialog.get_delete_partial();
            dialog.set_message("Removing…".into());
            let partial = if delete { unfinished.clone() } else { std::collections::HashSet::new() };
            start(partial, delete && is_all, Some(weak_dialog.clone()));
        });
        let weak_dialog = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
        });
        if let Some(previous) = remove_window.borrow_mut().take() {
            let _ = previous.hide();
        }
        if let Err(error) = dialog.show() {
            ui.set_message(error.to_string().into());
        }
        *remove_window.borrow_mut() = Some(dialog);
    });

    let lists_window = Rc::new(RefCell::new(None::<ListsWindow>));
    let ipc_lists = ipc.clone();
    let refresh_lists = refresh.clone();
    let weak = ui.as_weak();
    ui.on_transfer_lists(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let dialog = match ListsWindow::new() {
            Ok(dialog) => dialog,
            Err(error) => {
                ui.set_message(error.to_string().into());
                return;
            }
        };
        let cached = Rc::new(RefCell::new(
            None::<(String, String, xdm_ipc::TransferList, String)>,
        ));
        let cache_preview = cached.clone();
        let weak_dialog = dialog.as_weak();
        dialog.on_preview_list(move || {
            let Some(dialog) = weak_dialog.upgrade() else {
                return;
            };
            dialog.set_preview_ready(false);
            *cache_preview.borrow_mut() = None;
            let file = dialog.get_input_file().to_string();
            let directory = dialog.get_directory().to_string();
            if !std::path::Path::new(&directory).is_absolute() {
                dialog.set_message("Enter an absolute destination directory".into());
                return;
            }
            match xdm_transfer_list::read(std::path::Path::new(&file)) {
                Ok(list) => {
                    let text = list
                        .entries
                        .iter()
                        .map(|entry| {
                            format!(
                                "{} → {}\n{}",
                                entry.kind,
                                std::path::Path::new(&directory)
                                    .join(&entry.filename)
                                    .display(),
                                entry.url
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    dialog.set_preview(text.into());
                    dialog.set_message(
                        format!(
                            "{} tasks ready. Choose paused or add to queue.",
                            list.entries.len()
                        )
                        .into(),
                    );
                    dialog.set_preview_ready(true);
                    let id = format!(
                        "gui-list-{}-{}",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    );
                    *cache_preview.borrow_mut() = Some((file, directory, list, id));
                }
                Err(error) => {
                    dialog.set_preview("".into());
                    dialog.set_message(error.into());
                }
            }
        });
        let weak_dialog = dialog.as_weak();
        let ipc_import = ipc_lists.clone();
        let refresh_import = refresh_lists.clone();
        dialog.on_import_list(move |enqueue| {
            let Some(dialog) = weak_dialog.upgrade() else {
                return;
            };
            let cache = cached.borrow();
            let Some((file, directory, list, request_id)) = cache.as_ref() else {
                return;
            };
            if file != dialog.get_input_file().as_str()
                || directory != dialog.get_directory().as_str()
            {
                dialog.set_preview_ready(false);
                dialog.set_message("Preview again after changing the file or destination".into());
                return;
            }
            let command = Command::ImportList {
                request_id: request_id.clone(),
                directory: directory.clone(),
                entries: list.entries.clone(),
                enqueue,
            };
            dialog.set_message("Importing…".into());
            let weak_dialog = weak_dialog.clone();
            let refresh_import = refresh_import.clone();
            ipc_then(&ipc_import, move |paths| paths.request(command).map_err(|e| e.to_string()), move |result| {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                match result {
                    Ok(reply) if reply.ok => {
                        dialog.set_preview_ready(false);
                        dialog.set_message(
                            format!(
                                "Imported {} tasks {}",
                                reply.task_ids.len(),
                                if enqueue { "into the queue" } else { "paused" }
                            )
                            .into(),
                        );
                        refresh_import();
                    }
                    Ok(reply) => dialog.set_message(
                        reply.error.unwrap_or_else(|| "Import rejected".into()).into(),
                    ),
                    Err(error) => dialog.set_message(error.into()),
                }
            });
        });
        let weak_dialog = dialog.as_weak();
        let ipc_export = ipc_lists.clone();
        dialog.on_export_list(move |json| {
            let Some(dialog) = weak_dialog.upgrade() else {
                return;
            };
            let file = std::path::PathBuf::from(dialog.get_export_file().as_str());
            let format = if json { "json" } else { "urls" };
            dialog.set_message("Exporting…".into());
            let weak_dialog = weak_dialog.clone();
            ipc_then(&ipc_export, move |paths| xdm_transfer_list::export(paths, &file, format), move |result| {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                match result {
                    Ok(count) => dialog
                        .set_message(format!("Exported {count} tasks; request headers omitted").into()),
                    Err(error) => dialog.set_message(error.into()),
                }
            });
        });
        let weak_dialog = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
        });
        if let Some(previous) = lists_window.borrow_mut().take() {
            let _ = previous.hide();
        }
        if let Err(error) = dialog.show() {
            ui.set_message(error.to_string().into());
        }
        *lists_window.borrow_mut() = Some(dialog);
    });

    let settings_window = Rc::new(RefCell::new(None::<QueueSettingsWindow>));
    let weak_main_for_settings = ui.as_weak();
    let ipc_settings = ipc.clone();
    let refresh_main_for_settings = refresh.clone();
    let poll_completion_for_settings = poll_completion.clone();
    let open_queue_settings: Rc<dyn Fn(i64)> = {
        let settings_window = settings_window.clone();
        Rc::new(move |queue_id: i64| {
            let Some(main_ui) = weak_main_for_settings.upgrade() else { return; };
            let dialog = match QueueSettingsWindow::new() {
                Ok(dialog) => dialog,
                Err(error) => {
                    main_ui.set_message(format!("Failed to open queue settings: {error}").into());
                    return;
                }
            };
            let weak_dialog = dialog.as_weak();
            let ipc_reload = ipc_settings.clone();
            let reload: Arc<dyn Fn() + Send + Sync> = {
                let weak_dialog = weak_dialog.clone();
                Arc::new(move || {
                    let weak = weak_dialog.clone();
                    ipc_reload.execute(move |paths| {
                        let list_reply = paths.request(Command::List);
                        let events_reply = paths.request(Command::QueueEvents { queue_id });
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(dialog) = weak.upgrade() else { return; };
                            if let Ok(reply) = list_reply {
                                if let Some(queue) = reply.queues.iter().find(|q| q.id == queue_id) {
                                    dialog.set_queue_title(format!("Queue #{} — {}", queue.id, queue.name).into());
                                    dialog.set_status_text(format!(
                                        "Status: {} · Solo: {} · State: {} · Limit: {}",
                                        if queue.running { "running" } else { "stopped" },
                                        if queue.started_alone { "yes" } else { "no" },
                                        queue.run_state,
                                        queue.limit
                                    ).into());
                                    if queue.retry_max == -1 {
                                        dialog.set_retry_count("infinite".into());
                                        dialog.set_retry_infinite(true);
                                    } else {
                                        dialog.set_retry_count(queue.retry_max.to_string().into());
                                        dialog.set_retry_infinite(false);
                                    }
                                    dialog.set_retry_delay(queue.retry_delay.to_string().into());
                                    dialog.set_schedule_kind(queue.schedule_kind.clone().into());
                                    if queue.schedule_kind == "once" {
                                        dialog.set_once_start(scheduler::format_once_datetime(queue.schedule_start).into());
                                        dialog.set_once_stop(queue.schedule_stop.map(scheduler::format_once_datetime).unwrap_or_default().into());
                                    } else if queue.schedule_kind == "daily" {
                                        dialog.set_daily_start(scheduler::format_clock(queue.schedule_start).into());
                                        dialog.set_daily_stop(queue.schedule_stop.map(scheduler::format_clock).unwrap_or_default().into());
                                        let (m, tu, w, th, f, sa, su) = scheduler::unpack_days_mask(queue.schedule_days);
                                        dialog.set_day_mon(m);
                                        dialog.set_day_tue(tu);
                                        dialog.set_day_wed(w);
                                        dialog.set_day_thu(th);
                                        dialog.set_day_fri(f);
                                        dialog.set_day_sat(sa);
                                        dialog.set_day_sun(su);
                                    }
                                    if let Some(action) = CompletionAction::decode(&queue.completion_action) {
                                        match action {
                                            CompletionAction::None => dialog.set_completion_action("none".into()),
                                            CompletionAction::Notify => dialog.set_completion_action("notify".into()),
                                            CompletionAction::Sleep => dialog.set_completion_action("sleep".into()),
                                            CompletionAction::Hibernate => dialog.set_completion_action("hibernate".into()),
                                            CompletionAction::Shutdown => dialog.set_completion_action("shutdown".into()),
                                            CompletionAction::Restart => dialog.set_completion_action("restart".into()),
                                            CompletionAction::Exit => dialog.set_completion_action("exit".into()),
                                            CompletionAction::Program { program, args } => {
                                                dialog.set_completion_action("program".into());
                                                dialog.set_program_path(program.into());
                                                dialog.set_program_args(args.join("\n").into());
                                            }
                                        }
                                    } else {
                                        dialog.set_completion_action("none".into());
                                    }
                                    if dialog.get_countdown_seconds().trim().is_empty() {
                                        dialog.set_countdown_seconds("60".into());
                                    }
                                    dialog.set_confirm_destructive(false);
                                }
                            }
                            if let Ok(reply) = events_reply {
                                let event_strings = reply.queue_events.iter().map(scheduler::format_event).collect::<Vec<_>>();
                                dialog.set_events(ModelRc::new(VecModel::from(event_strings.into_iter().map(slint::SharedString::from).collect::<Vec<_>>())));
                            }
                        });
                    });
                })
            };

            let weak_dialog = dialog.as_weak();
            let ipc_action = ipc_settings.clone();
            let reload_action = reload.clone();
            let refresh_main = refresh_main_for_settings.clone();
            dialog.on_start_solo(move || {
                let weak = weak_dialog.clone();
                let reload = reload_action.clone();
                let refresh_main = refresh_main.clone();
                ipc_action.execute(move |paths| {
                    let res = request(paths, Command::QueueStartOne { queue_id });
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(dialog) = weak.upgrade() {
                            dialog.set_message(res.into());
                        }
                        reload();
                        refresh_main();
                    });
                });
            });

            let weak_dialog = dialog.as_weak();
            let ipc_action = ipc_settings.clone();
            let reload_action = reload.clone();
            let refresh_main = refresh_main_for_settings.clone();
            dialog.on_stop_solo(move || {
                let weak = weak_dialog.clone();
                let reload = reload_action.clone();
                let refresh_main = refresh_main.clone();
                ipc_action.execute(move |paths| {
                    let res = request(paths, Command::QueueStopOne { queue_id });
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(dialog) = weak.upgrade() {
                            dialog.set_message(res.into());
                        }
                        reload();
                        refresh_main();
                    });
                });
            });

            let reload_ev = reload.clone();
            dialog.on_refresh_events(move || {
                reload_ev();
            });

            let weak_dialog = dialog.as_weak();
            dialog.on_select_kind(move |kind| {
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.set_schedule_kind(kind);
                }
            });

            let weak_dialog = dialog.as_weak();
            dialog.on_select_weekdays(move |all| {
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.set_day_mon(true);
                    dialog.set_day_tue(true);
                    dialog.set_day_wed(true);
                    dialog.set_day_thu(true);
                    dialog.set_day_fri(true);
                    dialog.set_day_sat(all);
                    dialog.set_day_sun(all);
                }
            });

            let weak_dialog = dialog.as_weak();
            let ipc_retry = ipc_settings.clone();
            let reload_retry = reload.clone();
            let refresh_main = refresh_main_for_settings.clone();
            dialog.on_save_retry(move || {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                let count_str = dialog.get_retry_count().to_string();
                let infinite = dialog.get_retry_infinite();
                let max_retries = match scheduler::parse_retries(&count_str, infinite) {
                    Ok(c) => c,
                    Err(e) => {
                        dialog.set_message(e.into());
                        return;
                    }
                };
                let delay_seconds = match dialog.get_retry_delay().as_str().parse::<u64>() {
                    Ok(d) if d <= 86400 => d,
                    _ => {
                        dialog.set_message("Delay must be a positive number of seconds (<= 86400)".into());
                        return;
                    }
                };
                let weak = weak_dialog.clone();
                let reload = reload_retry.clone();
                let refresh_main = refresh_main.clone();
                ipc_retry.execute(move |paths| {
                    let res = request(paths, Command::QueueSetRetry { queue_id, max_retries, delay_seconds });
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(dialog) = weak.upgrade() {
                            dialog.set_message(format!("Retry policy updated: {res}").into());
                        }
                        reload();
                        refresh_main();
                    });
                });
            });

            let weak_dialog = dialog.as_weak();
            let ipc_sched = ipc_settings.clone();
            let reload_sched = reload.clone();
            let refresh_main = refresh_main_for_settings.clone();
            dialog.on_save_schedule(move || {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                let kind = dialog.get_schedule_kind().to_string();
                let command = match kind.as_str() {
                    "none" => Command::QueueSetSchedule {
                        queue_id,
                        kind: "none".into(),
                        start: 0,
                        stop: None,
                        days: 0,
                    },
                    "once" => {
                        let start = match scheduler::parse_once_datetime(dialog.get_once_start().as_str()) {
                            Ok(s) => s,
                            Err(e) => {
                                dialog.set_message(e.into());
                                return;
                            }
                        };
                        let stop_str = dialog.get_once_stop().to_string();
                        let stop = if stop_str.trim().is_empty() || stop_str.trim() == "-" {
                            None
                        } else {
                            match scheduler::parse_once_datetime(stop_str.as_str()) {
                                Ok(s) => Some(s),
                                Err(e) => {
                                    dialog.set_message(format!("Stop datetime: {e}").into());
                                    return;
                                }
                            }
                        };
                        Command::QueueSetSchedule {
                            queue_id,
                            kind: "once".into(),
                            start,
                            stop,
                            days: 0,
                        }
                    }
                    "daily" => {
                        let start = match scheduler::parse_clock(dialog.get_daily_start().as_str()) {
                            Ok(s) => s,
                            Err(e) => {
                                dialog.set_message(e.into());
                                return;
                            }
                        };
                        let stop_str = dialog.get_daily_stop().to_string();
                        let stop = if stop_str.trim().is_empty() || stop_str.trim() == "-" {
                            None
                        } else {
                            match scheduler::parse_clock(stop_str.as_str()) {
                                Ok(s) => Some(s),
                                Err(e) => {
                                    dialog.set_message(format!("Stop time: {e}").into());
                                    return;
                                }
                            }
                        };
                        let days = scheduler::days_mask(
                            dialog.get_day_mon(),
                            dialog.get_day_tue(),
                            dialog.get_day_wed(),
                            dialog.get_day_thu(),
                            dialog.get_day_fri(),
                            dialog.get_day_sat(),
                            dialog.get_day_sun(),
                        );
                        if days == 0 {
                            dialog.set_message("Select at least one weekday".into());
                            return;
                        }
                        Command::QueueSetSchedule {
                            queue_id,
                            kind: "daily".into(),
                            start,
                            stop,
                            days,
                        }
                    }
                    _ => {
                        dialog.set_message("Invalid schedule mode".into());
                        return;
                    }
                };
                let weak = weak_dialog.clone();
                let reload = reload_sched.clone();
                let refresh_main = refresh_main.clone();
                ipc_sched.execute(move |paths| {
                    let res = request(paths, command);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(dialog) = weak.upgrade() {
                            dialog.set_message(format!("Schedule updated: {res}").into());
                        }
                        reload();
                        refresh_main();
                    });
                });
            });

            let weak_dialog = dialog.as_weak();
            dialog.on_select_completion(move |action| {
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.set_completion_action(action);
                    dialog.set_confirm_destructive(false);
                }
            });

            let weak_dialog = dialog.as_weak();
            let ipc_comp = ipc_settings.clone();
            let reload_comp = reload.clone();
            let refresh_main = refresh_main_for_settings.clone();
            let poll_comp = poll_completion_for_settings.clone();
            dialog.on_save_completion(move || {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                let action_str = dialog.get_completion_action().to_string();
                let prog_path = dialog.get_program_path().to_string();
                let prog_args = dialog.get_program_args().to_string();
                let action = match scheduler::build_completion_action(&action_str, &prog_path, &prog_args) {
                    Ok(a) => a,
                    Err(e) => {
                        dialog.set_message(e.into());
                        return;
                    }
                };
                let confirmed = dialog.get_confirm_destructive();
                if let Err(e) = scheduler::validate_destructive_confirmation(&action, confirmed) {
                    dialog.set_message(e.into());
                    return;
                }
                let countdown = match scheduler::parse_countdown(dialog.get_countdown_seconds().as_str()) {
                    Ok(c) => c,
                    Err(e) => {
                        dialog.set_message(e.into());
                        return;
                    }
                };
                let is_destructive = action.is_destructive();
                let weak = weak_dialog.clone();
                let reload = reload_comp.clone();
                let refresh_main = refresh_main.clone();
                let poll = poll_comp.clone();
                ipc_comp.execute(move |paths| {
                    let res = request(paths, Command::QueueSetCompletionAction {
                        queue_id,
                        action,
                        confirm: is_destructive && confirmed,
                        countdown_seconds: Some(countdown),
                    });
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(dialog) = weak.upgrade() {
                            dialog.set_message(format!("Completion action set: {res}").into());
                        }
                        reload();
                        refresh_main();
                        poll();
                    });
                });
            });

            let weak_dialog = dialog.as_weak();
            let ipc_all = ipc_settings.clone();
            let reload_all = reload.clone();
            let refresh_main = refresh_main_for_settings.clone();
            let poll_all = poll_completion_for_settings.clone();
            dialog.on_save_all(move || {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                let count_str = dialog.get_retry_count().to_string();
                let infinite = dialog.get_retry_infinite();
                let max_retries = match scheduler::parse_retries(&count_str, infinite) {
                    Ok(c) => c,
                    Err(e) => {
                        dialog.set_message(e.into());
                        return;
                    }
                };
                let delay_seconds = match dialog.get_retry_delay().as_str().parse::<u64>() {
                    Ok(d) if d <= 86400 => d,
                    _ => {
                        dialog.set_message("Delay must be a positive number of seconds (<= 86400)".into());
                        return;
                    }
                };
                let kind = dialog.get_schedule_kind().to_string();
                let sched_cmd = match kind.as_str() {
                    "none" => Command::QueueSetSchedule {
                        queue_id,
                        kind: "none".into(),
                        start: 0,
                        stop: None,
                        days: 0,
                    },
                    "once" => {
                        let start = match scheduler::parse_once_datetime(dialog.get_once_start().as_str()) {
                            Ok(s) => s,
                            Err(e) => {
                                dialog.set_message(e.into());
                                return;
                            }
                        };
                        let stop_str = dialog.get_once_stop().to_string();
                        let stop = if stop_str.trim().is_empty() || stop_str.trim() == "-" {
                            None
                        } else {
                            match scheduler::parse_once_datetime(stop_str.as_str()) {
                                Ok(s) => Some(s),
                                Err(e) => {
                                    dialog.set_message(format!("Stop datetime: {e}").into());
                                    return;
                                }
                            }
                        };
                        Command::QueueSetSchedule {
                            queue_id,
                            kind: "once".into(),
                            start,
                            stop,
                            days: 0,
                        }
                    }
                    "daily" => {
                        let start = match scheduler::parse_clock(dialog.get_daily_start().as_str()) {
                            Ok(s) => s,
                            Err(e) => {
                                dialog.set_message(e.into());
                                return;
                            }
                        };
                        let stop_str = dialog.get_daily_stop().to_string();
                        let stop = if stop_str.trim().is_empty() || stop_str.trim() == "-" {
                            None
                        } else {
                            match scheduler::parse_clock(stop_str.as_str()) {
                                Ok(s) => Some(s),
                                Err(e) => {
                                    dialog.set_message(format!("Stop time: {e}").into());
                                    return;
                                }
                            }
                        };
                        let days = scheduler::days_mask(
                            dialog.get_day_mon(),
                            dialog.get_day_tue(),
                            dialog.get_day_wed(),
                            dialog.get_day_thu(),
                            dialog.get_day_fri(),
                            dialog.get_day_sat(),
                            dialog.get_day_sun(),
                        );
                        if days == 0 {
                            dialog.set_message("Select at least one weekday".into());
                            return;
                        }
                        Command::QueueSetSchedule {
                            queue_id,
                            kind: "daily".into(),
                            start,
                            stop,
                            days,
                        }
                    }
                    _ => {
                        dialog.set_message("Invalid schedule mode".into());
                        return;
                    }
                };
                let action_str = dialog.get_completion_action().to_string();
                let prog_path = dialog.get_program_path().to_string();
                let prog_args = dialog.get_program_args().to_string();
                let action = match scheduler::build_completion_action(&action_str, &prog_path, &prog_args) {
                    Ok(a) => a,
                    Err(e) => {
                        dialog.set_message(e.into());
                        return;
                    }
                };
                let confirmed = dialog.get_confirm_destructive();
                if let Err(e) = scheduler::validate_destructive_confirmation(&action, confirmed) {
                    dialog.set_message(e.into());
                    return;
                }
                let countdown = match scheduler::parse_countdown(dialog.get_countdown_seconds().as_str()) {
                    Ok(c) => c,
                    Err(e) => {
                        dialog.set_message(e.into());
                        return;
                    }
                };
                let is_destructive = action.is_destructive();
                let comp_cmd = Command::QueueSetCompletionAction {
                    queue_id,
                    action,
                    confirm: is_destructive && confirmed,
                    countdown_seconds: Some(countdown),
                };
                let weak = weak_dialog.clone();
                let reload = reload_all.clone();
                let refresh_main = refresh_main.clone();
                let poll = poll_all.clone();
                ipc_all.execute(move |paths| {
                    let _ = paths.request(Command::QueueSetRetry { queue_id, max_retries, delay_seconds });
                    let _ = paths.request(sched_cmd);
                    let _ = paths.request(comp_cmd);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(dialog) = weak.upgrade() {
                            dialog.set_message("All settings successfully saved".into());
                        }
                        reload();
                        refresh_main();
                        poll();
                    });
                });
            });

            let weak_dialog = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(dialog) = weak_dialog.upgrade() {
                    let _ = dialog.hide();
                }
            });

            reload();
            if let Some(previous) = settings_window.borrow_mut().take() {
                let _ = previous.hide();
            }
            if let Err(error) = dialog.show() {
                main_ui.set_message(error.to_string().into());
            }
            *settings_window.borrow_mut() = Some(dialog);
        })
    };

    let weak_main = ui.as_weak();
    let open_settings_from_main = open_queue_settings.clone();
    ui.on_queue_settings(move || {
        let Some(ui) = weak_main.upgrade() else { return; };
        let queue_id = ui.get_queue_filter_id().as_str().parse::<i64>().unwrap_or(0);
        let target = if queue_id > 0 { queue_id } else { 1 };
        open_settings_from_main(target);
    });

    let queues_window = Rc::new(RefCell::new(None::<QueuesWindow>));
    let ipc_queues = ipc.clone();
    let sel_main_queues = selected_task_ids.clone();
    let all_main_queues = table_shared.all.clone();
    let refresh_main_queues = refresh.clone();
    let ids_main_queues = task_ids.clone();
    let weak_main = ui.as_weak();
    let open_settings_for_queues = open_queue_settings.clone();
    ui.on_manage_queues(move || {
        let Some(ui) = weak_main.upgrade() else { return; };
        let assign_target = resolve_target(&ui, &ids_main_queues, &sel_main_queues, &all_main_queues);
        let dialog = match QueuesWindow::new() {
            Ok(dialog) => dialog,
            Err(error) => { ui.set_message(error.to_string().into()); return; }
        };
        dialog.set_can_assign(assign_target.count() > 0);
        let rows = Arc::new(Mutex::new(Vec::<xdm_ipc::QueueSummary>::new()));
        let preferred = Arc::new(Mutex::new(None::<i64>));
        let rows_refresh = rows.clone();
        let preferred_refresh = preferred.clone();
        let ipc_refresh = ipc_queues.clone();
        let weak = dialog.as_weak();
        let refresh_queues: Arc<dyn Fn() + Send + Sync> = {
            let weak = weak.clone();
            Arc::new(move || {
                let Some(dialog) = weak.upgrade() else { return; };
                let previous = preferred_refresh.lock().ok().and_then(|mut guard| guard.take()).or_else(|| usize::try_from(dialog.get_selected()).ok().and_then(|index| rows_refresh.lock().ok().and_then(|guard| guard.get(index).map(|queue| queue.id))));
                let weak = weak.clone();
                let rows_refresh = rows_refresh.clone();
                ipc_refresh.execute(move |paths| {
                    let result = paths.request(Command::List).map_err(|e| e.to_string());
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(dialog) = weak.upgrade() else { return; };
                        match result {
                            Ok(reply) if reply.ok => {
                                let selected = previous.and_then(|id| reply.queues.iter().position(|queue| queue.id == id));
                                let labels = reply.queues.iter().map(|queue| {
                                    let solo = if queue.started_alone { " [solo]" } else { "" };
                                    let sync = scheduler::format_sync_label(queue);
                                    format!("#{}  {}  {}{solo}{sync}  {}/{} active  {} queued  {} paused", queue.id, queue.name, if queue.running {"enabled"} else {"stopped"}, queue.active, queue.limit, queue.queued, queue.paused).into()
                                }).collect::<Vec<slint::SharedString>>();
                                dialog.set_queues(ModelRc::new(VecModel::from(labels)));
                                dialog.set_selected(selected.map_or(-1, |index| index as i32));
                                if let Some(queue) = selected.and_then(|index| reply.queues.get(index)) {
                                    dialog.set_name(queue.name.clone().into()); dialog.set_limit(queue.limit.to_string().into());
                                }
                                dialog.set_message(if reply.queue_running == Some(true) { "Global queue running. Each enabled queue uses its own job limit." } else { "Global queue stopped. Use Start all in the main window to run enabled queues." }.into());
                                if let Ok(mut guard) = rows_refresh.lock() {
                                    *guard = reply.queues;
                                }
                            }
                            Ok(reply) => dialog.set_message(reply.error.unwrap_or_else(|| "Queue list failed".into()).into()),
                            Err(error) => dialog.set_message(error.into()),
                        }
                    });
                });
            })
        };
        let weak = dialog.as_weak();
        let rows_select = rows.clone();
        dialog.on_select_queue(move |index| {
            if let (Some(dialog), Some(queue)) = (weak.upgrade(), usize::try_from(index).ok().and_then(|index| rows_select.lock().ok().and_then(|guard| guard.get(index).cloned()))) {
                dialog.set_name(queue.name.into()); dialog.set_limit(queue.limit.to_string().into());
                if let Some(interval) = queue.sync_interval { dialog.set_sync_interval(interval.to_string().into()); }
            }
        });
        let refresh_button = refresh_queues.clone();
        dialog.on_refresh_queues(move || refresh_button());

        let weak_dialog = dialog.as_weak();
        let open_settings = open_settings_for_queues.clone();
        let rows_settings = rows.clone();
        dialog.on_open_settings(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let queue_id = usize::try_from(dialog.get_selected())
                .ok()
                .and_then(|index| rows_settings.lock().ok().and_then(|guard| guard.get(index).map(|queue| queue.id)))
                .unwrap_or(1);
            open_settings(queue_id);
        });

        let ipc_action = ipc_queues.clone();
        let refresh_action = refresh_queues.clone();
        let refresh_main = refresh_main_queues.clone();
        let weak_dialog = dialog.as_weak();
        let weak_main = ui.as_weak();
        dialog.on_action(move |action| {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let queue_id = usize::try_from(dialog.get_selected()).ok().and_then(|index| rows.lock().ok().and_then(|guard| guard.get(index).map(|queue| queue.id)));
            if action != 0 && action != 10 && queue_id.is_none() { dialog.set_message("Select a queue first".into()); return; }
            let queue_id = queue_id.unwrap_or(1);
            if action == 7 {
                if let Some(ui) = weak_main.upgrade() { ui.set_queue_filter_id(queue_id.to_string().into()); }
                refresh_main(); let _ = dialog.hide(); return;
            }
            let limit = if action == 0 || action == 2 || action == 10 {
                match dialog.get_limit().as_str().parse::<usize>() { Ok(limit) => limit, Err(_) => { dialog.set_message("Enter a job limit from 1 to 16".into()); return; } }
            } else { 2 };
            let command = match action {
                0 => Command::QueueCreate { name: dialog.get_name().to_string(), limit },
                1 => Command::QueueRename { queue_id, name: dialog.get_name().to_string() },
                2 => Command::QueueSetConcurrency { queue_id, limit },
                3 => Command::QueueSetRunning { queue_id, running: true },
                4 => Command::QueueSetRunning { queue_id, running: false },
                5 => Command::QueueRemove { queue_id },
                6 => match &assign_target {
                    Target::All(all) => bulk_command(all, xdm_ipc::BulkAction::AssignQueue { queue_id }),
                    Target::Ids(ids) if !ids.is_empty() && ids.len() <= 500 => Command::AssignQueue { queue_id, task_ids: ids.clone() },
                    Target::Ids(ids) if !ids.is_empty() => Command::TaskBulk {
                        action: xdm_ipc::BulkAction::AssignQueue { queue_id },
                        selection: xdm_ipc::BulkSelection::Ids(ids.clone()),
                        expected_total: Some(ids.len() as u64),
                    },
                    Target::Ids(_) => return,
                },
                8 => Command::QueueStartOne { queue_id },
                9 => Command::QueueStopOne { queue_id },
                10 => Command::QueueCreateSync { name: dialog.get_name().to_string(), limit },
                11 => match dialog.get_sync_interval().trim().parse::<u64>() {
                    Ok(seconds) if seconds >= 1 => Command::QueueSetSync { queue_id, interval_seconds: Some(seconds) },
                    _ => { dialog.set_message("Enter a check interval in whole seconds (at least 1)".into()); return; }
                },
                12 => Command::QueueSetSync { queue_id, interval_seconds: None },
                13 => Command::QueueCheck { queue_id },
                _ => return,
            };
            let note = match action {
                10 => "Synchronization queue created",
                11 => "Sync interval set",
                12 => "Automatic checks off; use Check now",
                13 => "Check started; the result appears under Scheduler & Settings > Recent Queue Events",
                _ => "",
            };
            dialog.set_message("Working…".into());
            let weak_dialog = weak_dialog.clone();
            let preferred = preferred.clone();
            let refresh_action = refresh_action.clone();
            let refresh_main = refresh_main.clone();
            let bulk_assign = action == 6 && matches!(command, Command::TaskBulk { .. });
            ipc_then(&ipc_action, move |paths| paths.request(command).map_err(|e| e.to_string()), move |result| {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                match result {
                    Ok(reply) if reply.ok => {
                        if let Ok(mut guard) = preferred.lock() {
                            *guard = reply.queue_id;
                        }
                        refresh_action(); refresh_main();
                        if bulk_assign {
                            dialog.set_message(bulk_message(Ok(reply), "Moved").into());
                        } else if !note.is_empty() {
                            dialog.set_message(note.into());
                        }
                    }
                    Ok(reply) => dialog.set_message(reply.error.unwrap_or_else(|| "Queue command rejected".into()).into()),
                    Err(error) => dialog.set_message(error.into()),
                }
            });
        });
        let weak = dialog.as_weak();
        dialog.on_dismiss(move || { if let Some(dialog) = weak.upgrade() { let _ = dialog.hide(); } });
        refresh_queues();
        if let Some(previous) = queues_window.borrow_mut().take() { let _ = previous.hide(); }
        if let Err(error) = dialog.show() { ui.set_message(error.to_string().into()); }
        *queues_window.borrow_mut() = Some(dialog);
    });

    let batch_window = Rc::new(RefCell::new(None::<BatchWindow>));
    let weak_main_batch = ui.as_weak();
    let ipc_batch = ipc.clone();
    let refresh_batch = refresh.clone();
    let paths_dir_batch = paths.dir.clone();
    ui.on_open_batch(move || {
        let Some(main_ui) = weak_main_batch.upgrade() else { return; };
        let dialog = match BatchWindow::new() {
            Ok(d) => d,
            Err(e) => { main_ui.set_message(e.to_string().into()); return; }
        };
        dialog.set_directory(paths_dir_batch.join("downloads").to_string_lossy().to_string().into());
        let weak_dialog = dialog.as_weak();
        dialog.on_update_preview(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                let preview = dialog_helpers::preview_batch(dialog.get_template().as_str());
                dialog.set_preview_count(preview.count.to_string().into());
                dialog.set_preview_first(preview.first.into());
                dialog.set_preview_last(preview.last.into());
                dialog.set_can_submit(preview.valid);
                dialog.set_message(preview.message.into());
            }
        });
        let weak_dialog = dialog.as_weak();
        let weak_main_submit = weak_main_batch.clone();
        let ipc_submit = ipc_batch.clone();
        let refresh_submit = refresh_batch.clone();
        dialog.on_submit(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let template = dialog.get_template().to_string();
            let directory = dialog.get_directory().to_string();
            let start_now = dialog.get_start_now();
            if directory.trim().is_empty() {
                dialog.set_message("Enter an absolute destination directory".into());
                return;
            }
            let entries = match dialog_helpers::build_batch_entries(&template) {
                Ok(e) => e,
                Err(err) => {
                    dialog.set_message(err.into());
                    return;
                }
            };
            let request_id = format!(
                "gui-batch-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            let weak_d = weak_dialog.clone();
            let weak_m = weak_main_submit.clone();
            let refresh = refresh_submit.clone();
            ipc_submit.execute(move |paths| {
                let res = paths.request(Command::ImportList {
                    request_id,
                    directory,
                    entries,
                    enqueue: start_now,
                });
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(dialog) = weak_d.upgrade() {
                        match res {
                            Ok(reply) if reply.ok => {
                                let _ = dialog.hide();
                                refresh();
                                if let Some(main) = weak_m.upgrade() {
                                    main.set_message(
                                        format!(
                                            "Imported {} batch tasks {}",
                                            reply.task_ids.len(),
                                            if start_now { "into queue" } else { "paused" }
                                        )
                                        .into(),
                                    );
                                }
                            }
                            Ok(reply) => {
                                dialog.set_message(
                                    reply.error.unwrap_or_else(|| "Batch import rejected".into()).into(),
                                );
                            }
                            Err(e) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                        }
                    }
                });
            });
        });
        let weak_dialog = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
        });
        if let Some(prev) = batch_window.borrow_mut().take() {
            let _ = prev.hide();
        }
        if let Err(e) = dialog.show() {
            main_ui.set_message(e.to_string().into());
        }
        *batch_window.borrow_mut() = Some(dialog);
    });

    let grabber_window = Rc::new(RefCell::new(None::<GrabberWindow>));
    let weak_main_grabber = ui.as_weak();
    let ipc_grabber = ipc.clone();
    let refresh_grabber = refresh.clone();
    let paths_dir_grabber = paths.dir.clone();
    ui.on_open_grabber(move || {
        let Some(main_ui) = weak_main_grabber.upgrade() else { return; };
        let dialog = match GrabberWindow::new() {
            Ok(d) => d,
            Err(e) => { main_ui.set_message(e.to_string().into()); return; }
        };
        dialog.set_directory(paths_dir_grabber.join("downloads").to_string_lossy().to_string().into());
        let saved_records = Arc::new(Mutex::new(Vec::<xdm_ipc::GrabProjectRecord>::new()));
        let records_store = saved_records.clone();
        let records_load = saved_records.clone();
        let crawl_timer = Rc::new(RefCell::new(None::<Timer>));

        let weak_dialog = dialog.as_weak();
        let ipc_projects = ipc_grabber.clone();
        let reload_projects: Arc<dyn Fn() + Send + Sync> = {
            let weak_dialog = weak_dialog.clone();
            let records_store = records_store.clone();
            Arc::new(move || {
                let weak = weak_dialog.clone();
                let records_store = records_store.clone();
                ipc_projects.execute(move |paths| {
                    let res = paths.request(Command::GrabProjects);
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(dialog) = weak.upgrade() else { return; };
                        if let Ok(reply) = res {
                            if reply.ok {
                                let records = reply.grab_projects;
                                let labels = records.iter().map(|rec| {
                                    let url = rec.project.get("start_url").and_then(|u| u.as_str()).unwrap_or("");
                                    format!("{}  ({})", rec.name, url)
                                }).collect::<Vec<_>>();
                                dialog.set_saved_projects(ModelRc::new(VecModel::from(
                                    labels.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()
                                )));
                                if let Ok(mut guard) = records_store.lock() {
                                    *guard = records;
                                }
                            }
                        }
                    });
                });
            })
        };

        let reload_p = reload_projects.clone();
        dialog.on_refresh_projects(move || {
            reload_p();
        });

        let weak_dialog = dialog.as_weak();
        dialog.on_load_project(move |index| {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            if let Ok(guard) = records_load.lock() {
                if let Some(record) = usize::try_from(index).ok().and_then(|i| guard.get(i)) {
                    dialog.set_project_name(record.name.clone().into());
                    if let Ok(proj) = serde_json::from_value::<xdm_grabber::Project>(record.project.clone()) {
                        dialog.set_start_url(proj.start_url.into());
                        dialog.set_depth(proj.max_depth.to_string().into());
                        dialog.set_max_pages(proj.max_pages.to_string().into());
                        dialog.set_file_types(proj.file_types.join(", ").into());
                        dialog.set_include_patterns(proj.include_patterns.join(", ").into());
                        dialog.set_exclude_patterns(proj.exclude_patterns.join(", ").into());
                        dialog.set_same_host(proj.same_host_only);
                        dialog.set_obey_robots(proj.obey_robots);
                        dialog.set_message(format!("Loaded saved project '{}'", record.name).into());
                    }
                }
            }
        });

        let weak_dialog = dialog.as_weak();
        let ipc_save = ipc_grabber.clone();
        let reload_after_save = reload_projects.clone();
        dialog.on_save_project(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let name = dialog.get_project_name().trim().to_string();
            if name.is_empty() {
                dialog.set_message("Enter a project name to save".into());
                return;
            }
            let project = match dialog_helpers::build_grabber_project(
                dialog.get_start_url().as_str(),
                dialog.get_depth().as_str(),
                dialog.get_max_pages().as_str(),
                dialog.get_file_types().as_str(),
                dialog.get_include_patterns().as_str(),
                dialog.get_exclude_patterns().as_str(),
                dialog.get_same_host(),
                dialog.get_obey_robots(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    dialog.set_message(e.into());
                    return;
                }
            };
            let project_val = match serde_json::to_value(&project) {
                Ok(v) => v,
                Err(e) => {
                    dialog.set_message(e.to_string().into());
                    return;
                }
            };
            let weak = weak_dialog.clone();
            let reload = reload_after_save.clone();
            ipc_save.execute(move |paths| {
                let res = paths.request(Command::GrabSave { name: name.clone(), project: project_val });
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(dialog) = weak.upgrade() {
                        match res {
                            Ok(reply) if reply.ok => {
                                dialog.set_message(format!("Project '{name}' saved successfully").into());
                                reload();
                            }
                            Ok(reply) => {
                                dialog.set_message(reply.error.unwrap_or_else(|| "Failed to save project".into()).into());
                            }
                            Err(e) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                        }
                    }
                });
            });
        });

        let weak_dialog = dialog.as_weak();
        let ipc_remove = ipc_grabber.clone();
        let reload_after_remove = reload_projects.clone();
        dialog.on_remove_project(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let name = dialog.get_project_name().trim().to_string();
            if name.is_empty() {
                dialog.set_message("Enter or select a project name to remove".into());
                return;
            }
            let weak = weak_dialog.clone();
            let reload = reload_after_remove.clone();
            ipc_remove.execute(move |paths| {
                let res = paths.request(Command::GrabRemove { name: name.clone() });
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(dialog) = weak.upgrade() {
                        match res {
                            Ok(reply) if reply.ok => {
                                dialog.set_message(format!("Project '{name}' removed").into());
                                reload();
                            }
                            Ok(reply) => {
                                dialog.set_message(reply.error.unwrap_or_else(|| "Failed to remove project".into()).into());
                            }
                            Err(e) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                        }
                    }
                });
            });
        });

        let weak_dialog = dialog.as_weak();
        let ipc_crawl = ipc_grabber.clone();
        let refresh_main_crawl = refresh_grabber.clone();
        let timer_crawl = crawl_timer.clone();
        dialog.on_start_crawl(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let project = match dialog_helpers::build_grabber_project(
                dialog.get_start_url().as_str(),
                dialog.get_depth().as_str(),
                dialog.get_max_pages().as_str(),
                dialog.get_file_types().as_str(),
                dialog.get_include_patterns().as_str(),
                dialog.get_exclude_patterns().as_str(),
                dialog.get_same_host(),
                dialog.get_obey_robots(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    dialog.set_message(e.into());
                    return;
                }
            };
            let directory = dialog.get_directory().to_string();
            if directory.trim().is_empty() {
                dialog.set_message("Enter an absolute destination directory".into());
                return;
            }
            let enqueue = dialog.get_enqueue();
            let project_val = match serde_json::to_value(&project) {
                Ok(v) => v,
                Err(e) => {
                    dialog.set_message(e.to_string().into());
                    return;
                }
            };
            let request_id = format!(
                "gui-grab-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            dialog.set_is_crawling(true);
            dialog.set_status_text("Starting crawl…".into());

            let poll_timer = Timer::default();
            let req_id_poll = request_id.clone();
            let weak_dialog_poll = weak_dialog.clone();
            let ipc_poll = ipc_crawl.clone();
            let refresh_poll = refresh_main_crawl.clone();
            poll_timer.start(TimerMode::Repeated, Duration::from_millis(1000), move || {
                let Some(dialog) = weak_dialog_poll.upgrade() else { return; };
                if !dialog.get_is_crawling() {
                    return;
                }
                let req_id = req_id_poll.clone();
                let weak = weak_dialog_poll.clone();
                let refresh = refresh_poll.clone();
                ipc_poll.execute(move |paths| {
                    let status_res = paths.request(Command::GrabStatus { request_id: req_id });
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(dialog) = weak.upgrade() else { return; };
                        if let Ok(reply) = status_res {
                            if let Some(grab) = reply.grab {
                                dialog.set_status_text(format!(
                                    "State: {} · Pages: {} · Files: {} · Errors: {}",
                                    grab.state, grab.pages, grab.files, grab.errors
                                ).into());
                                if grab.state == "done" || grab.state == "failed" {
                                    dialog.set_is_crawling(false);
                                    refresh();
                                    dialog.set_message(
                                        format!("Crawl {}: {} files found, {} pages crawled", grab.state, grab.files, grab.pages).into()
                                    );
                                }
                            }
                        }
                    });
                });
            });
            *timer_crawl.borrow_mut() = Some(poll_timer);

            let weak_d = weak_dialog.clone();
            ipc_crawl.execute(move |paths| {
                let res = paths.request(Command::GrabSite {
                    request_id,
                    project: project_val,
                    directory,
                    queue_id: None,
                    enqueue,
                });
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(dialog) = weak_d.upgrade() else { return; };
                    match res {
                        Ok(reply) if reply.ok => {
                            dialog.set_status_text("Crawl running…".into());
                        }
                        Ok(reply) => {
                            dialog.set_is_crawling(false);
                            dialog.set_message(reply.error.unwrap_or_else(|| "GrabSite rejected".into()).into());
                        }
                        Err(e) => {
                            dialog.set_is_crawling(false);
                            dialog.set_message(format!("Daemon unavailable: {e}").into());
                        }
                    }
                });
            });
        });

        let weak_dialog = dialog.as_weak();
        let timer_stop = crawl_timer.clone();
        dialog.on_stop_crawl(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.set_is_crawling(false);
                *timer_stop.borrow_mut() = None;
                dialog.set_status_text("Crawl stopped".into());
            }
        });

        let weak_dialog = dialog.as_weak();
        let timer_dismiss = crawl_timer.clone();
        dialog.on_dismiss(move || {
            *timer_dismiss.borrow_mut() = None;
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
        });

        reload_projects();
        if let Some(prev) = grabber_window.borrow_mut().take() {
            let _ = prev.hide();
        }
        if let Err(e) = dialog.show() {
            main_ui.set_message(e.to_string().into());
        }
        *grabber_window.borrow_mut() = Some(dialog);
    });

    let categories_window = Rc::new(RefCell::new(None::<CategoriesWindow>));
    let weak_main_cat = ui.as_weak();
    let ipc_cat = ipc.clone();
    let refresh_cat = refresh.clone();
    ui.on_open_categories(move || {
        let Some(main_ui) = weak_main_cat.upgrade() else { return; };
        let dialog = match CategoriesWindow::new() {
            Ok(d) => d,
            Err(e) => { main_ui.set_message(e.to_string().into()); return; }
        };
        let category_rules = Arc::new(Mutex::new(Vec::<xdm_categories::CategoryRule>::new()));
        let rules_store = category_rules.clone();
        let rules_load = category_rules.clone();

        let weak_dialog = dialog.as_weak();
        let ipc_list = ipc_cat.clone();
        let reload_categories: Arc<dyn Fn() + Send + Sync> = {
            let weak_dialog = weak_dialog.clone();
            let rules_store = rules_store.clone();
            Arc::new(move || {
                let weak = weak_dialog.clone();
                let rules_store = rules_store.clone();
                ipc_list.execute(move |paths| {
                    let res = paths.request(Command::CategoryList);
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(dialog) = weak.upgrade() else { return; };
                        if let Ok(reply) = res {
                            if reply.ok {
                                let formatted = reply.categories.iter().map(dialog_helpers::format_category_rule).collect::<Vec<_>>();
                                dialog.set_categories(ModelRc::new(VecModel::from(
                                    formatted.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()
                                )));
                                if let Ok(mut guard) = rules_store.lock() {
                                    *guard = reply.categories;
                                }
                            }
                        }
                    });
                });
            })
        };

        let reload_c = reload_categories.clone();
        dialog.on_refresh_categories(move || {
            reload_c();
        });

        let weak_dialog = dialog.as_weak();
        dialog.on_select_category(move |index| {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            if let Ok(guard) = rules_load.lock() {
                if let Some(rule) = usize::try_from(index).ok().and_then(|i| guard.get(i)) {
                    dialog.set_name(rule.name.clone().into());
                    dialog.set_extensions(rule.extensions.join(", ").into());
                    dialog.set_destination(rule.destination.clone().into());
                    dialog.set_sites(rule.sites.join(", ").into());
                }
            }
        });

        let weak_dialog = dialog.as_weak();
        let ipc_save = ipc_cat.clone();
        let reload_after_save = reload_categories.clone();
        let refresh_main_save = refresh_cat.clone();
        dialog.on_save_category(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let name = dialog.get_name().trim().to_string();
            if name.is_empty() {
                dialog.set_message("Enter a category name".into());
                return;
            }
            let extensions = dialog_helpers::parse_extensions(dialog.get_extensions().as_str());
            let destination = dialog.get_destination().trim().to_string();
            let sites = dialog_helpers::parse_sites(dialog.get_sites().as_str());
            let weak = weak_dialog.clone();
            let reload = reload_after_save.clone();
            let refresh = refresh_main_save.clone();
            ipc_save.execute(move |paths| {
                let res = paths.request(Command::CategorySave {
                    name: name.clone(),
                    extensions,
                    destination,
                    sites,
                });
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(dialog) = weak.upgrade() {
                        match res {
                            Ok(reply) if reply.ok => {
                                dialog.set_message(format!("Category '{name}' saved successfully").into());
                                reload();
                                refresh();
                            }
                            Ok(reply) => {
                                dialog.set_message(reply.error.unwrap_or_else(|| "Failed to save category".into()).into());
                            }
                            Err(e) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                        }
                    }
                });
            });
        });

        let weak_dialog = dialog.as_weak();
        let ipc_remove = ipc_cat.clone();
        let reload_after_remove = reload_categories.clone();
        let refresh_main_remove = refresh_cat.clone();
        dialog.on_remove_category(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let name = dialog.get_name().trim().to_string();
            if name.is_empty() {
                dialog.set_message("Select or enter a category name to remove".into());
                return;
            }
            let weak = weak_dialog.clone();
            let reload = reload_after_remove.clone();
            let refresh = refresh_main_remove.clone();
            ipc_remove.execute(move |paths| {
                let res = paths.request(Command::CategoryRemove { name: name.clone() });
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(dialog) = weak.upgrade() {
                        match res {
                            Ok(reply) if reply.ok => {
                                dialog.set_message(format!("Category '{name}' removed").into());
                                reload();
                                refresh();
                            }
                            Ok(reply) => {
                                dialog.set_message(reply.error.unwrap_or_else(|| "Cannot remove category".into()).into());
                            }
                            Err(e) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                        }
                    }
                });
            });
        });

        let weak_dialog = dialog.as_weak();
        dialog.on_clear_fields(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.set_name("".into());
                dialog.set_extensions("".into());
                dialog.set_destination("".into());
                dialog.set_sites("".into());
                dialog.set_selected_category(-1);
                dialog.set_message("Fields cleared. Enter details for new category.".into());
            }
        });

        let weak_dialog = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
        });

        reload_categories();
        if let Some(prev) = categories_window.borrow_mut().take() {
            let _ = prev.hide();
        }
        if let Err(e) = dialog.show() {
            main_ui.set_message(e.to_string().into());
        }
        *categories_window.borrow_mut() = Some(dialog);
    });

    let connection_window = Rc::new(RefCell::new(None::<ConnectionWindow>));
    let weak_main_conn = ui.as_weak();
    let ipc_conn = ipc.clone();
    let settings_conn = gui_settings.clone();
    let paths_dir_conn = paths.dir.clone();
    ui.on_open_connection(move || {
        let Some(main_ui) = weak_main_conn.upgrade() else { return; };
        let dialog = match ConnectionWindow::new() {
            Ok(d) => d,
            Err(e) => { main_ui.set_message(e.to_string().into()); return; }
        };
        let weak_dialog = dialog.as_weak();
        let settings_c = settings_conn.clone();
        let ipc_load = ipc_conn.clone();
        let reload_connection: Arc<dyn Fn() + Send + Sync> = {
            let weak_dialog = weak_dialog.clone();
            let settings_c = settings_c.clone();
            Arc::new(move || {
                let weak = weak_dialog.clone();
                let conns = settings_c.lock().map(|s| s.default_connections).unwrap_or(8);
                ipc_load.execute(move |paths| {
                    let res = paths.request(Command::GetProxy);
                    let stall = paths.request(Command::GetStallTimeout).ok().and_then(|reply| reply.stall_timeout_seconds);
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(dialog) = weak.upgrade() else { return; };
                        dialog.set_default_connections(conns.to_string().into());
                        if let Some(seconds) = stall {
                            dialog.set_stall_timeout(seconds.to_string().into());
                        }
                        if let Ok(reply) = res {
                            if let Some(proxy_val) = reply.proxy {
                                if let Ok(choice) = serde_json::from_value::<xdm_ipc::ProxyChoice>(proxy_val) {
                                    match choice {
                                        xdm_ipc::ProxyChoice::System => dialog.set_proxy_mode("system".into()),
                                        xdm_ipc::ProxyChoice::Direct => dialog.set_proxy_mode("direct".into()),
                                        xdm_ipc::ProxyChoice::Manual { http, https, bypass } => {
                                            dialog.set_proxy_mode("manual".into());
                                            dialog.set_http_proxy(http.unwrap_or_default().into());
                                            dialog.set_https_proxy(https.unwrap_or_default().into());
                                            dialog.set_bypass_list(bypass.join(", ").into());
                                        }
                                    }
                                }
                            }
                        }
                    });
                });
            })
        };

        let weak_dialog = dialog.as_weak();
        dialog.on_select_mode(move |mode| {
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.set_proxy_mode(mode);
            }
        });

        let weak_dialog = dialog.as_weak();
        let ipc_save = ipc_conn.clone();
        let settings_save = settings_conn.clone();
        let paths_dir_save = paths_dir_conn.clone();
        dialog.on_save_settings(move || {
            let Some(dialog) = weak_dialog.upgrade() else { return; };
            let conns_str = dialog.get_default_connections().to_string();
            let conns = match conns_str.trim().parse::<usize>() {
                Ok(c) if (1..=16).contains(&c) => c,
                _ => {
                    dialog.set_message("Default connections must be a number between 1 and 16".into());
                    return;
                }
            };
            let stall_seconds = match dialog.get_stall_timeout().trim().parse::<u64>() {
                Ok(seconds) if (1..=600).contains(&seconds) => seconds,
                _ => {
                    dialog.set_message("Stall timeout must be a number of seconds between 1 and 600".into());
                    return;
                }
            };
            let mode = dialog.get_proxy_mode().to_string();
            let http = dialog.get_http_proxy().to_string();
            let https = dialog.get_https_proxy().to_string();
            let bypass = dialog.get_bypass_list().to_string();
            let proxy_choice = match dialog_helpers::build_proxy_choice(&mode, &http, &https, &bypass) {
                Ok(p) => p,
                Err(e) => {
                    dialog.set_message(e.into());
                    return;
                }
            };
            if let Ok(mut guard) = settings_save.lock() {
                guard.default_connections = conns;
                let _ = settings::save_settings(&paths_dir_save, &guard);
            }
            let weak = weak_dialog.clone();
            ipc_save.execute(move |paths| {
                let res = paths.request(Command::SetProxy { proxy: proxy_choice });
                let stall_res = match &res {
                    Ok(reply) if reply.ok => Some(paths.request(Command::SetStallTimeout { seconds: stall_seconds })),
                    _ => None,
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(dialog) = weak.upgrade() {
                        match res {
                            Ok(reply) if reply.ok => match stall_res {
                                Some(Ok(stall)) if !stall.ok => {
                                    dialog.set_message(stall.error.unwrap_or_else(|| "Stall timeout rejected".into()).into());
                                }
                                Some(Err(e)) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                                _ => dialog.set_message("Connection and proxy settings saved successfully".into()),
                            },
                            Ok(reply) => {
                                dialog.set_message(reply.error.unwrap_or_else(|| "Proxy setting rejected".into()).into());
                            }
                            Err(e) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                        }
                    }
                });
            });
        });

        let reload_c = reload_connection.clone();
        dialog.on_refresh_settings(move || {
            reload_c();
        });
        let open_bandwidth_conn = open_bandwidth.clone();
        dialog.on_open_bandwidth(move || open_bandwidth_conn());

        let weak_dialog = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
        });

        reload_connection();
        if let Some(prev) = connection_window.borrow_mut().take() {
            let _ = prev.hide();
        }
        if let Err(e) = dialog.show() {
            main_ui.set_message(e.to_string().into());
        }
        *connection_window.borrow_mut() = Some(dialog);
    });

    let add_url_window = Rc::new(RefCell::new(None::<AddUrlWindow>));
    let weak_main_url = ui.as_weak();
    let ipc_add_url_dlg = ipc.clone();
    let refresh_add_url_dlg = refresh.clone();
    let settings_add_url_dlg = gui_settings.clone();
    let paths_dir_add_url = paths.dir.clone();
    let add_url_win_holder = add_url_window.clone();

    let open_add_url_fn: Rc<dyn Fn(Option<String>)> = {
        let add_url_window = add_url_win_holder.clone();
        let weak_main_url = weak_main_url.clone();
        let ipc_submit = ipc_add_url_dlg.clone();
        let refresh_submit = refresh_add_url_dlg.clone();
        let settings_submit = settings_add_url_dlg.clone();
        let paths_dir_submit = paths_dir_add_url.clone();
        Rc::new(move |prefill_url: Option<String>| {
            let Some(main_ui) = weak_main_url.upgrade() else { return; };
            let dialog = match AddUrlWindow::new() {
                Ok(d) => d,
                Err(e) => { main_ui.set_message(e.to_string().into()); return; }
            };
            if let Some(ref u) = prefill_url {
                dialog.set_url(u.clone().into());
                let filename = u
                    .split('/')
                    .last()
                    .and_then(|p| p.split('?').next())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("download.bin");
                let destination = paths_dir_submit.join("downloads").join(filename).to_string_lossy().to_string();
                dialog.set_destination(destination.into());
            }
            let weak_dialog = dialog.as_weak();
            let weak_main_submit = weak_main_url.clone();
            let ipc_submit = ipc_submit.clone();
            let refresh_submit = refresh_submit.clone();
            let settings_submit = settings_submit.clone();
            let paths_dir_submit = paths_dir_submit.clone();
            dialog.on_submit(move || {
                let Some(dialog) = weak_dialog.upgrade() else { return; };
                let raw_url = dialog.get_url().to_string();
                let url = match dialog_helpers::validate_download_url(&raw_url) {
                    Ok(u) => u,
                    Err(err) => {
                        dialog.set_message(err.into());
                        return;
                    }
                };
                let mut destination = dialog.get_destination().trim().to_string();
                if destination.is_empty() {
                    let filename = url
                        .split('/')
                        .last()
                        .and_then(|p| p.split('?').next())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("download.bin");
                    destination = paths_dir_submit.join("downloads").join(filename).to_string_lossy().to_string();
                }
                let connections = settings_submit
                    .lock()
                    .map(|g| g.default_connections)
                    .unwrap_or(settings::DEFAULT_CONNECTIONS);
                let request_id = format!(
                    "gui-add-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                );
                let weak_d = weak_dialog.clone();
                let weak_m = weak_main_submit.clone();
                let refresh = refresh_submit.clone();
                ipc_submit.execute(move |paths| {
                    let res = paths.request(Command::Submit {
                        request_id,
                        url,
                        output: destination,
                        headers: Vec::new(),
                        priority: 1,
                        connections,
                    });
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(dialog) = weak_d.upgrade() {
                            match res {
                                Ok(reply) if reply.ok => {
                                    let _ = dialog.hide();
                                    refresh();
                                    if let Some(main) = weak_m.upgrade() {
                                        main.set_message(
                                            format!("Task #{} accepted", reply.task_id.unwrap_or_default()).into()
                                        );
                                    }
                                }
                                Ok(reply) => {
                                    dialog.set_message(reply.error.unwrap_or_else(|| "Download rejected".into()).into());
                                }
                                Err(e) => dialog.set_message(format!("Daemon unavailable: {e}").into()),
                            }
                        }
                    });
                });
            });

            let weak_dialog = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(dialog) = weak_dialog.upgrade() {
                    let _ = dialog.hide();
                }
            });

            if let Some(prev) = add_url_window.borrow_mut().take() {
                let _ = prev.hide();
            }
            if let Err(e) = dialog.show() {
                main_ui.set_message(e.to_string().into());
            }
            *add_url_window.borrow_mut() = Some(dialog);
        })
    };

    let open_add_for_ui = open_add_url_fn.clone();
    ui.on_open_add_url(move || {
        open_add_for_ui(None);
    });

    let media_window = Rc::new(RefCell::new(None::<MediaWindow>));
    let weak_main_media = ui.as_weak();
    let ipc_media_dlg = ipc.clone();
    let refresh_media_dlg = refresh.clone();
    let paths_dir_media = paths.dir.clone();
    let live_ids_media = live_task_ids.clone();
    let media_win_holder = media_window.clone();
    let settings_media_dlg = gui_settings.clone();

    ui.on_open_add_media(move || {
        let Some(main_ui) = weak_main_media.upgrade() else { return; };
        let dialog = match MediaWindow::new() {
            Ok(d) => d,
            Err(e) => { main_ui.set_message(e.to_string().into()); return; }
        };
        let current_theme = settings_media_dlg.lock().map(|s| s.theme.clone()).unwrap_or_default();
        dialog.invoke_apply_theme(current_theme.into());

        let tracks_cache = Arc::new(Mutex::new(Vec::<xdm_media::Track>::new()));
        let is_live_flag = Arc::new(AtomicBool::new(false));

        // on_inspect
        let weak_d = dialog.as_weak();
        let tracks_inspect = tracks_cache.clone();
        let is_live_inspect = is_live_flag.clone();
        let paths_dir_inspect = paths_dir_media.clone();
        dialog.on_inspect(move || {
            let Some(dialog) = weak_d.upgrade() else { return; };
            let url = dialog.get_url().trim().to_string();
            if url.is_empty() {
                dialog.set_message("Please enter a manifest URL".into());
                return;
            }
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                dialog.set_message("URL must start with http:// or https://".into());
                return;
            }
            let headers = dialog_helpers::parse_header_lines(dialog.get_headers().as_str());
            dialog.set_inspecting(true);
            dialog.set_message("Inspecting manifest off UI thread…".into());

            let weak_thread = weak_d.clone();
            let tracks_thread = tracks_inspect.clone();
            let is_live_thread = is_live_inspect.clone();
            let paths_dir_thread = paths_dir_inspect.clone();
            std::thread::spawn(move || {
                let report_res = xdm_media::inspect_report_with_headers(&url, &headers);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(dialog) = weak_thread.upgrade() else { return; };
                    dialog.set_inspecting(false);
                    match report_res {
                        Ok(inspection) => {
                            is_live_thread.store(inspection.live, Ordering::SeqCst);
                            dialog.set_is_live(inspection.live);
                            let track_items: Vec<MediaTrackItem> = inspection
                                .tracks
                                .iter()
                                .map(|t| MediaTrackItem {
                                    text: dialog_helpers::format_media_track(t).into(),
                                })
                                .collect();
                            dialog.set_tracks(ModelRc::new(VecModel::from(track_items)));

                            let first_video = inspection.tracks.iter().find(|t| t.kind.eq_ignore_ascii_case("video")).map(|t| t.index);
                            let first_audio = inspection.tracks.iter().find(|t| t.kind.eq_ignore_ascii_case("audio")).map(|t| t.index);
                            let first_sub = inspection.tracks.iter().find(|t| t.kind.eq_ignore_ascii_case("subtitle")).map(|t| t.index);

                            if let Some(v) = first_video {
                                dialog.set_video_track(v.to_string().into());
                            }
                            if let Some(a) = first_audio {
                                dialog.set_audio_track(a.to_string().into());
                            }
                            if let Some(s) = first_sub {
                                dialog.set_subtitles_tracks(s.to_string().into());
                            }

                            let mode_str = dialog.get_mode().to_string();
                            let mode_res = dialog_helpers::build_media_mode(
                                &mode_str,
                                first_video,
                                first_audio,
                                &dialog.get_subtitles_tracks().to_string(),
                                dialog.get_audio_format().as_str(),
                                dialog.get_audio_bitrate().as_str(),
                                dialog.get_max_minutes().as_str(),
                                inspection.live,
                            );

                            if let Ok(m) = mode_res {
                                if dialog.get_destination().trim().is_empty() {
                                    let filename = dialog_helpers::suggest_media_filename(&url, &m);
                                    let dest = paths_dir_thread.join("downloads").join(filename);
                                    dialog.set_destination(dest.to_string_lossy().to_string().into());
                                }
                            }

                            let live_msg = if inspection.live {
                                " (Live stream detected — Live recording available)"
                            } else {
                                ""
                            };
                            dialog.set_message(
                                format!("Inspection successful: {} track(s) found.{}", inspection.tracks.len(), live_msg).into(),
                            );
                            if let Ok(mut guard) = tracks_thread.lock() {
                                *guard = inspection.tracks;
                            }
                        }
                        Err(err) => {
                            dialog.set_message(format!("Inspection failed: {err}").into());
                        }
                    }
                });
            });
        });

        // on_set_mode
        let weak_d = dialog.as_weak();
        let paths_dir_mode = paths_dir_media.clone();
        dialog.on_set_mode(move |new_mode| {
            let Some(dialog) = weak_d.upgrade() else { return; };
            dialog.set_mode(new_mode.clone());
            let dest = dialog.get_destination().trim().to_string();
            let new_ext = match new_mode.as_str() {
                "video" => dialog.get_output_container().to_string(),
                "audio" => match dialog.get_audio_format().as_str() {
                    "original" => "m4a".to_string(),
                    other => other.to_string(),
                },
                "subtitles" => "srt".to_string(),
                "live" => "mp4".to_string(),
                _ => "mp4".to_string(),
            };
            if !dest.is_empty() {
                let p = std::path::PathBuf::from(dest);
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("media");
                let parent = p.parent().unwrap_or_else(|| std::path::Path::new(""));
                let new_path = parent.join(format!("{stem}.{new_ext}"));
                dialog.set_destination(new_path.to_string_lossy().to_string().into());
            } else {
                let url = dialog.get_url().to_string();
                let stem = dialog_helpers::suggest_media_filename(&url, &xdm_ipc::MediaMode::Video { video: 0, audio: None, subtitles: vec![] });
                let p = std::path::PathBuf::from(stem);
                let stem_str = p.file_stem().and_then(|s| s.to_str()).unwrap_or("media");
                let new_dest = paths_dir_mode.join("downloads").join(format!("{stem_str}.{new_ext}"));
                dialog.set_destination(new_dest.to_string_lossy().to_string().into());
            }
        });

        // on_set_audio_format
        let weak_d = dialog.as_weak();
        dialog.on_set_audio_format(move |new_fmt| {
            let Some(dialog) = weak_d.upgrade() else { return; };
            dialog.set_audio_format(new_fmt.clone());
            let dest = dialog.get_destination().trim().to_string();
            if !dest.is_empty() {
                let new_ext = match new_fmt.as_str() {
                    "original" => "m4a",
                    "mp3" => "mp3",
                    "m4a" => "m4a",
                    "opus" => "opus",
                    _ => "mp3",
                };
                let p = std::path::PathBuf::from(dest);
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
                let parent = p.parent().unwrap_or_else(|| std::path::Path::new(""));
                let new_path = parent.join(format!("{stem}.{new_ext}"));
                dialog.set_destination(new_path.to_string_lossy().to_string().into());
            }
        });

        // on_select_track
        let weak_d = dialog.as_weak();
        let tracks_select = tracks_cache.clone();
        dialog.on_select_track(move |idx| {
            let Some(dialog) = weak_d.upgrade() else { return; };
            if let Ok(guard) = tracks_select.lock() {
                if let Some(track) = usize::try_from(idx).ok().and_then(|i| guard.get(i)) {
                    match track.kind.to_ascii_lowercase().as_str() {
                        "video" => {
                            dialog.set_video_track(track.index.to_string().into());
                            if dialog.get_mode() != "video" && dialog.get_mode() != "live" {
                                dialog.set_mode("video".into());
                            }
                        }
                        "audio" => {
                            dialog.set_audio_track(track.index.to_string().into());
                            if dialog.get_mode() != "audio" && dialog.get_mode() != "video" && dialog.get_mode() != "live" {
                                dialog.set_mode("audio".into());
                            }
                        }
                        "subtitle" => {
                            dialog.set_subtitles_tracks(track.index.to_string().into());
                            if dialog.get_mode() != "subtitles" && dialog.get_mode() != "video" {
                                dialog.set_mode("subtitles".into());
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        // on_submit
        let weak_d = dialog.as_weak();
        let weak_m = weak_main_media.clone();
        let ipc_submit = ipc_media_dlg.clone();
        let refresh_submit = refresh_media_dlg.clone();
        let live_ids_submit = live_ids_media.clone();
        dialog.on_submit(move || {
            let Some(dialog) = weak_d.upgrade() else { return; };
            let url = dialog.get_url().trim().to_string();
            if url.is_empty() {
                dialog.set_message("Manifest URL is required".into());
                return;
            }
            let headers = dialog_helpers::parse_header_lines(dialog.get_headers().as_str());
            let mode_str = dialog.get_mode().to_string();
            let video_idx = dialog.get_video_track().trim().parse::<usize>().ok();
            let audio_idx = dialog.get_audio_track().trim().parse::<usize>().ok();
            let subtitles_str = dialog.get_subtitles_tracks().to_string();
            let audio_format = dialog.get_audio_format().to_string();
            let audio_bitrate = dialog.get_audio_bitrate().to_string();
            let max_minutes = dialog.get_max_minutes().to_string();
            let is_live = dialog.get_is_live();

            let mode = match dialog_helpers::build_media_mode(
                &mode_str,
                video_idx,
                audio_idx,
                &subtitles_str,
                &audio_format,
                &audio_bitrate,
                &max_minutes,
                is_live,
            ) {
                Ok(m) => m,
                Err(e) => {
                    dialog.set_message(e.into());
                    return;
                }
            };

            let dest_str = dialog.get_destination().trim().to_string();
            let dest_path = match dialog_helpers::validate_media_destination(&dest_str, &mode) {
                Ok(p) => p,
                Err(e) => {
                    dialog.set_message(e.into());
                    return;
                }
            };

            let request_id = format!(
                "gui-media-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );

            let weak_thread = weak_d.clone();
            let weak_main = weak_m.clone();
            let refresh_thread = refresh_submit.clone();
            let live_ids_thread = live_ids_submit.clone();
            let is_live_job = matches!(mode, xdm_ipc::MediaMode::Live { .. });

            ipc_submit.execute(move |paths| {
                let res = paths.request(Command::SubmitMediaJob {
                    request_id,
                    url,
                    output: dest_path.to_string_lossy().to_string(),
                    headers,
                    mode,
                    priority: 1,
                });
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(dialog) = weak_thread.upgrade() {
                        match res {
                            Ok(reply) if reply.ok => {
                                if let Some(task_id) = reply.task_id {
                                    if is_live_job {
                                        if let Ok(mut guard) = live_ids_thread.lock() {
                                            guard.insert(task_id);
                                        }
                                    }
                                    if let Some(main) = weak_main.upgrade() {
                                        main.set_message(format!("Media task #{task_id} accepted").into());
                                    }
                                }
                                let _ = dialog.hide();
                                refresh_thread();
                            }
                            Ok(reply) => {
                                dialog.set_message(
                                    reply.error.unwrap_or_else(|| "Media submission rejected".into()).into(),
                                );
                            }
                            Err(e) => {
                                dialog.set_message(format!("Daemon unavailable: {e}").into());
                            }
                        }
                    }
                });
            });
        });

        // on_dismiss
        let weak_d = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(dialog) = weak_d.upgrade() {
                let _ = dialog.hide();
            }
        });

        if let Some(prev) = media_win_holder.borrow_mut().take() {
            let _ = prev.hide();
        }
        if let Err(e) = dialog.show() {
            main_ui.set_message(e.to_string().into());
        }
        *media_win_holder.borrow_mut() = Some(dialog);
    });

    let preferences_window = Rc::new(RefCell::new(None::<PreferencesWindow>));
    let weak_main_pref = ui.as_weak();
    let settings_pref = gui_settings.clone();
    let paths_dir_pref = paths.dir.clone();

    ui.on_open_preferences(move || {
        let Some(main_ui) = weak_main_pref.upgrade() else { return; };
        let dialog = match PreferencesWindow::new() {
            Ok(d) => d,
            Err(e) => { main_ui.set_message(e.to_string().into()); return; }
        };
        let current = settings_pref.lock().unwrap().clone();
        install_tr!(dialog);
        dialog.set_theme(current.theme.clone().into());
        dialog.set_language(current.language.clone().into());
        dialog.set_confirm_remove(current.confirm_remove);
        dialog.set_start_minimized(current.start_minimized);
        dialog.set_monitor_clipboard(current.monitor_clipboard);
        dialog.set_clipboard_extensions(current.clipboard_extensions_str().into());
        dialog.set_sound_download_complete(current.sound_download_complete);
        dialog.set_sound_download_complete_path(current.sound_download_complete_path.into());
        dialog.set_sound_download_failed(current.sound_download_failed);
        dialog.set_sound_download_failed_path(current.sound_download_failed_path.into());
        dialog.set_sound_queue_complete(current.sound_queue_complete);
        dialog.set_sound_queue_complete_path(current.sound_queue_complete_path.into());

        dialog.invoke_apply_theme(current.theme.into());

        let weak_dialog = dialog.as_weak();
        let weak_main = weak_main_pref.clone();
        dialog.on_select_theme(move |theme| {
            if let Some(d) = weak_dialog.upgrade() {
                d.set_theme(theme.clone());
                d.invoke_apply_theme(theme.clone());
            }
            if let Some(m) = weak_main.upgrade() {
                m.invoke_apply_theme(theme);
            }
        });

        let weak_dialog_lang = dialog.as_weak();
        let weak_main_lang = weak_main_pref.clone();
        dialog.on_select_language(move |lang| {
            if let Some(d) = weak_dialog_lang.upgrade() {
                d.set_language(lang.clone());
            }
            i18n::set_language(lang.as_str());
            if let Some(d) = weak_dialog_lang.upgrade() {
                d.global::<Tr>().set_lang(i18n::current_language().into());
            }
            if let Some(m) = weak_main_lang.upgrade() {
                m.global::<Tr>().set_lang(i18n::current_language().into());
            }
        });
        let weak_main_cols = weak_main_pref.clone();
        dialog.on_open_columns(move || {
            if let Some(m) = weak_main_cols.upgrade() {
                m.invoke_open_columns();
            }
        });

        let weak_dialog_sound = dialog.as_weak();
        let settings_test_sound = settings_pref.clone();
        dialog.on_test_sound(move |event_str| {
            let Some(d) = weak_dialog_sound.upgrade() else { return; };
            let event = match event_str.as_str() {
                "failed" => sound::SoundEvent::DownloadFailed,
                "queue" => sound::SoundEvent::QueueComplete,
                _ => sound::SoundEvent::DownloadComplete,
            };
            let mut temp_settings = settings_test_sound.lock().unwrap().clone();
            match event {
                sound::SoundEvent::DownloadComplete => {
                    temp_settings.sound_download_complete = true;
                    temp_settings.sound_download_complete_path = d.get_sound_download_complete_path().to_string();
                }
                sound::SoundEvent::DownloadFailed => {
                    temp_settings.sound_download_failed = true;
                    temp_settings.sound_download_failed_path = d.get_sound_download_failed_path().to_string();
                }
                sound::SoundEvent::QueueComplete => {
                    temp_settings.sound_queue_complete = true;
                    temp_settings.sound_queue_complete_path = d.get_sound_queue_complete_path().to_string();
                }
            }
            sound::trigger_event_sound(&temp_settings, event);
        });

        let weak_dialog_save = dialog.as_weak();
        let weak_main_save = weak_main_pref.clone();
        let settings_save = settings_pref.clone();
        let paths_dir_save = paths_dir_pref.clone();
        dialog.on_save_preferences(move || {
            let Some(d) = weak_dialog_save.upgrade() else { return; };
            let mut new_settings = settings_save.lock().unwrap().clone();
            new_settings.theme = d.get_theme().to_string();
            new_settings.language = d.get_language().to_string();
            new_settings.confirm_remove = d.get_confirm_remove();
            new_settings.start_minimized = d.get_start_minimized();
            new_settings.monitor_clipboard = d.get_monitor_clipboard();
            new_settings.set_clipboard_extensions_from_str(&d.get_clipboard_extensions());
            new_settings.sound_download_complete = d.get_sound_download_complete();
            new_settings.sound_download_complete_path = d.get_sound_download_complete_path().trim().to_string();
            new_settings.sound_download_failed = d.get_sound_download_failed();
            new_settings.sound_download_failed_path = d.get_sound_download_failed_path().trim().to_string();
            new_settings.sound_queue_complete = d.get_sound_queue_complete();
            new_settings.sound_queue_complete_path = d.get_sound_queue_complete_path().trim().to_string();

            if let Err(err) = settings::save_settings(&paths_dir_save, &new_settings) {
                d.set_message(format!("Failed to save settings: {err}").into());
                return;
            }
            *settings_save.lock().unwrap() = new_settings.clone();
            d.invoke_apply_theme(new_settings.theme.clone().into());
            if let Some(m) = weak_main_save.upgrade() {
                m.invoke_apply_theme(new_settings.theme.into());
                m.set_message("Preferences saved".into());
            }
            d.set_message("Preferences saved successfully".into());
        });

        let weak_dialog_dismiss = dialog.as_weak();
        dialog.on_dismiss(move || {
            if let Some(d) = weak_dialog_dismiss.upgrade() {
                let _ = d.hide();
            }
        });

        if let Some(prev) = preferences_window.borrow_mut().take() {
            let _ = prev.hide();
        }
        if let Err(e) = dialog.show() {
            main_ui.set_message(e.to_string().into());
        }
        *preferences_window.borrow_mut() = Some(dialog);
    });

    let clipboard_prompt_window = Rc::new(RefCell::new(None::<ClipboardPromptWindow>));
    let weak_main_prompt = ui.as_weak();
    let prompt_win_holder = clipboard_prompt_window.clone();
    let open_add_for_prompt = open_add_url_fn.clone();
    ui.on_show_clipboard_prompt(move |url_val| {
        let Some(main_ui) = weak_main_prompt.upgrade() else { return; };
        let prompt = match ClipboardPromptWindow::new() {
            Ok(p) => p,
            Err(e) => {
                main_ui.set_message(e.to_string().into());
                return;
            }
        };
        prompt.set_url(url_val.clone());
        let weak_prompt = prompt.as_weak();
        let open_dlg = open_add_for_prompt.clone();
        let url_to_add = url_val.to_string();
        prompt.on_add_download(move || {
            if let Some(p) = weak_prompt.upgrade() {
                let _ = p.hide();
            }
            open_dlg(Some(url_to_add.clone()));
        });
        let weak_prompt_dismiss = prompt.as_weak();
        prompt.on_dismiss(move || {
            if let Some(p) = weak_prompt_dismiss.upgrade() {
                let _ = p.hide();
            }
        });

        if let Some(prev) = prompt_win_holder.borrow_mut().take() {
            let _ = prev.hide();
        }
        if let Err(e) = prompt.show() {
            main_ui.set_message(e.to_string().into());
        }
        *prompt_win_holder.borrow_mut() = Some(prompt);
    });

    let clipboard_timer = Timer::default();
    let settings_clipboard = gui_settings.clone();
    let weak_main_clipboard = ui.as_weak();
    let last_seen_url = Arc::new(Mutex::new(None::<String>));

    if let Some(initial_clip) = clipboard::read_clipboard_system() {
        if let Ok(settings_guard) = settings_clipboard.lock() {
            if let Some(matched) = clipboard::extract_matching_url(&initial_clip, &settings_guard.clipboard_extensions) {
                *last_seen_url.lock().unwrap() = Some(matched);
            }
        }
    }

    clipboard_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let (enabled, extensions) = match settings_clipboard.lock() {
            Ok(s) => (s.monitor_clipboard, s.clipboard_extensions.clone()),
            Err(_) => return,
        };
        if !enabled {
            return;
        }

        let last_seen = last_seen_url.clone();
        let weak_main = weak_main_clipboard.clone();

        std::thread::spawn(move || {
            let Some(clip_text) = clipboard::read_clipboard_system() else { return; };
            let Some(matched_url) = clipboard::extract_matching_url(&clip_text, &extensions) else { return; };

            let mut last_seen_guard = match last_seen.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if last_seen_guard.as_deref() == Some(&matched_url) {
                return;
            }
            *last_seen_guard = Some(matched_url.clone());
            drop(last_seen_guard);

            let prompt_url = matched_url;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(main_ui) = weak_main.upgrade() {
                    main_ui.invoke_show_clipboard_prompt(prompt_url.into());
                }
            });
        });
    });

    refresh();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(2), move || {
        refresh()
    });
    poll_completion();
    let comp_timer = Timer::default();
    let poll_for_timer = poll_completion.clone();
    let has_pending_for_timer = has_pending_action.clone();
    let comp_tick = std::sync::atomic::AtomicU64::new(0);
    comp_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let tick = comp_tick.fetch_add(1, Ordering::SeqCst);
        if has_pending_for_timer.load(Ordering::SeqCst) || tick % 5 == 0 {
            poll_for_timer();
        }
    });
    ui.run()?;
    Ok(())
}

#[cfg(test)]
mod table_perf_tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::time::Instant;

    fn summaries(first: usize, count: usize) -> Vec<xdm_ipc::TaskSummary> {
        (first..first + count)
            .map(|i| {
                serde_json::from_value(serde_json::json!({
                    "id": i,
                    "url": format!("http://example.invalid/dir/file{i:05}.bin"),
                    "output": format!("/tmp/xdm/file{i:05}.bin"),
                    "status": if i % 50 == 0 { "downloading" } else { "queued" },
                    "priority": 1,
                    "error": null,
                    "downloaded": 10,
                    "total": 1_000_000u64,
                    "category": "Other",
                    "queue_id": 1,
                    "bytes_per_second": 1234,
                    "eta_seconds": 77,
                    "created_at": "2026-01-01 00:00:00",
                    "updated_at": "2026-01-02 00:00:00",
                }))
                .expect("task summary fixture")
            })
            .collect()
    }

    /// Row conversion for one 200-row page plus installing it into a 20,000-row
    /// model: the per-page work done on the UI thread.
    #[test]
    fn paged_install_of_20k_task_table_is_fast() {
        let queues: HashMap<i64, String> = HashMap::from([(1, "Main".to_string())]);
        let model = PagedModel::<TaskRow>::new(loading_row());
        let start = Instant::now();
        for page in 0..3 {
            let rows: Vec<TaskRow> = summaries(page * PAGE + 1, PAGE)
                .into_iter()
                .map(|task| row_from_summary(task, &queues, &HashSet::new()))
                .collect();
            assert!(model.apply(0, 20_000, vec![(page, rows)], false));
        }
        let took = start.elapsed();
        eprintln!("[perf] 3 pages x {PAGE} rows installed into a 20000-row model in {took:?}");
        assert_eq!(model.row_count(), 20_000);
        assert_eq!(model.row_data(PAGE + 5).unwrap().id, (PAGE + 6) as i32);
        assert_eq!(model.row_data(19_999).unwrap().id, 0, "unloaded rows are placeholders");
        assert!(took.as_millis() < 500, "installing pages took {took:?}");
    }

    #[test]
    fn rows_show_quota_wait_and_restart_required_clearly() {
        let queues = HashMap::new();
        let mut task = summaries(1, 1).remove(0);
        task.status = "quota_wait".into();
        assert_eq!(row_from_summary(task.clone(), &queues, &HashSet::new()).status.as_str(), "Quota wait");
        task.status = "restart_required".into();
        assert_eq!(row_from_summary(task, &queues, &HashSet::new()).status.as_str(), "Needs restart");
    }

    #[test]
    fn sort_columns_map_to_daemon_keys() {
        assert_eq!(daemon_sort("filename", true, false).as_deref(), Some("output"));
        assert_eq!(daemon_sort("filename", false, false).as_deref(), Some("-output"));
        assert_eq!(daemon_sort("id", true, true).as_deref(), Some("position"));
        assert_eq!(daemon_sort("last_try_date", false, false).as_deref(), Some("-updated"));
        assert_eq!(daemon_sort("size", true, false).as_deref(), Some("size"));
        assert_eq!(daemon_sort("time_left", false, false).as_deref(), Some("-progress"));
        assert_eq!(daemon_sort("transfer_rate", true, false).as_deref(), Some("rate"));
        assert_eq!(daemon_sort("queue", true, false).as_deref(), Some("queue"));
        assert_eq!(daemon_sort("bogus", true, false), None);
    }
}
