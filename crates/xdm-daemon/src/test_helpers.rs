#![cfg(test)]
#![allow(dead_code)]

use crate::os_actions::CompletionAdapter;
use crate::schedule::tests::FakeClock;
use crate::{Error, Service, queue_summaries};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use xdm_ipc::{Command, QueueSummary, Reply};

pub(crate) struct Counting(pub(crate) AtomicUsize);

impl CompletionAdapter for Counting {
    fn run(&self, _queue_id: i64, _action: &str, _armed: bool) -> String {
        self.0.fetch_add(1, Ordering::Relaxed);
        "fake".into()
    }
}

pub(crate) fn service(now: i64) -> (Arc<Service>, Arc<FakeClock>, Arc<Counting>) {
    let clock = Arc::new(FakeClock::new(now, 0));
    let adapter = Arc::new(Counting(AtomicUsize::new(0)));
    let service = Service::open_db(
        std::path::Path::new(":memory:"),
        clock.clone(),
        adapter.clone(),
    )
    .unwrap();
    (service, clock, adapter)
}

pub(crate) fn submit(service: &Service, name: &str) -> i64 {
    service
        .command(Command::Submit {
            request_id: name.into(),
            url: format!("http://127.0.0.1:1/{name}"),
            output: format!("/nonexistent-xdm-test/{name}"),
            headers: Vec::new(),
            priority: 1,
            connections: 1,
        })
        .unwrap()
        .task_id
        .unwrap()
}

pub(crate) fn queue(service: &Service, name: &str) -> i64 {
    service
        .command(Command::QueueCreate {
            name: name.into(),
            limit: 1,
        })
        .unwrap()
        .queue_id
        .unwrap()
}

pub(crate) fn submit_request(
    service: &Service,
    name: &str,
    method: &str,
    body: Option<&[u8]>,
) -> Reply {
    service
        .command(Command::SubmitRequest {
            request_id: name.into(),
            url: format!("http://127.0.0.1:1/{name}"),
            output: format!("/nonexistent-xdm-test/{name}"),
            headers: Vec::new(),
            method: method.into(),
            body: body.map(<[u8]>::to_vec),
            priority: 1,
            connections: 1,
        })
        .unwrap()
}

pub(crate) fn stored_details(service: &Service, id: i64) -> (String, Option<Vec<u8>>) {
    let inner = service.inner.lock().unwrap();
    inner
        .db
        .query_row("SELECT method,body FROM tasks WHERE id=?1", [id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
}

pub(crate) fn summary(service: &Service, queue_id: i64) -> QueueSummary {
    let inner = service.inner.lock().unwrap();
    queue_summaries(&inner)
        .unwrap()
        .into_iter()
        .find(|row| row.id == queue_id)
        .unwrap()
}

pub(crate) fn order(service: &Service, queue_id: i64) -> Vec<i64> {
    let inner = service.inner.lock().unwrap();
    inner
        .db
        .prepare("SELECT id FROM tasks WHERE queue_id=?1 ORDER BY priority DESC,position,id")
        .unwrap()
        .query_map([queue_id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

pub(crate) fn finish(service: &Service, id: i64, status: &str, error: Option<&str>) {
    let mut inner = service.inner.lock().unwrap();
    inner
        .db
        .execute("UPDATE tasks SET status='running' WHERE id=?1", [id])
        .unwrap();
    inner
        .finish_task(id, status, error.map(str::to_string), None, None)
        .unwrap();
}

pub(crate) fn task_state(service: &Service, id: i64) -> (String, i64, Option<i64>, i64) {
    let inner = service.inner.lock().unwrap();
    inner
        .db
        .query_row(
            "SELECT status,retry_count,retry_at,queue_id FROM tasks WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
}

pub(crate) fn inner_resume(service: &Service, id: i64) {
    let inner = service.inner.lock().unwrap();
    inner
        .db
        .execute(
            "UPDATE tasks SET status='queued',retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
            [id],
        )
        .unwrap();
}

pub(crate) fn auth_error(status: u16, realm: Option<&str>) -> Error {
    Box::new(xdm_http::AuthRequired(xdm_http::AuthChallenge {
        status,
        scheme: "Basic".into(),
        realm: realm.map(str::to_owned),
    }))
}
