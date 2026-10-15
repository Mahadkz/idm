use crate::credentials;
use crate::{Result, Service};
use rusqlite::OptionalExtension;
use std::io;
use url::Url;
use xdm_ipc::Reply;

/// Origin-level login pattern for a task URL, so a remembered login covers
/// the whole host (with its explicit port when one was given).
pub(crate) fn origin_pattern(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // `host_str` keeps IPv6 brackets, which the pattern parser accepts.
    let host = url.host_str()?;
    let authority = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    Some(format!("{}://{authority}/", url.scheme()))
}

impl Service {
    pub(crate) fn login_list(&self) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let mut reply = Reply::list(Vec::new());
        reply.logins = inner.credentials.summaries();
        Ok(reply)
    }

    pub(crate) fn login_save(
        &self,
        pattern: String,
        username: String,
        password: String,
        enabled: bool,
    ) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if let Err(error) = inner.credentials.save(&pattern, &username, &password, enabled) {
            return Ok(Reply::error(error.to_string()));
        }
        let mut reply = Reply::list(Vec::new());
        reply.logins = inner.credentials.summaries();
        Ok(reply)
    }

    pub(crate) fn login_remove(&self, id: i64) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if !inner.credentials.remove(id)? {
            return Ok(Reply::error("login not found"));
        }
        let mut reply = Reply::accepted(id);
        reply.logins = inner.credentials.summaries();
        Ok(reply)
    }

    /// Stores the proxy credentials and requeues tasks parked on a 407.
    pub(crate) fn proxy_login(&self, username: String, password: String) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if let Err(error) = inner.credentials.set_proxy_login(&username, &password) {
            return Ok(Reply::error(error.to_string()));
        }
        let requeued = inner.db.execute(
            "UPDATE tasks SET status='queued',error=NULL,error_kind=NULL,auth_realm=NULL,auth_proxy=0,retry_count=0,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE status='auth_required' AND auth_proxy=1",
            [],
        )?;
        drop(inner);
        if requeued > 0 {
            self.wake.notify_one();
        }
        Ok(Reply::accepted(requeued as i64))
    }

    pub(crate) fn task_provide_login(
        &self,
        task_id: i64,
        username: String,
        password: String,
        remember: bool,
    ) -> Result<Reply> {
        if !credentials::valid_login_fields(&username, &password) {
            return Ok(Reply::error(
                "username needs 1–128 printable bytes and the password 0–1024",
            ));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let Some((url, status)) = inner
            .db
            .query_row(
                "SELECT url,status FROM tasks WHERE id=?1",
                [task_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        else {
            return Ok(Reply::error("task not found"));
        };
        if status != "auth_required" {
            return Ok(Reply::error("task is not waiting for a login"));
        }
        let waits_on_proxy: bool = inner
            .db
            .query_row("SELECT auth_proxy FROM tasks WHERE id=?1", [task_id], |row| {
                row.get::<_, i64>(0).map(|value| value != 0)
            })?;
        if waits_on_proxy {
            return Ok(Reply::error(
                "the proxy asked for credentials; supply them with `daemon proxy-login`",
            ));
        }
        if remember {
            let Some(pattern) = origin_pattern(&url) else {
                return Ok(Reply::error("logins apply to http and https tasks only"));
            };
            if let Err(error) = inner.credentials.save(&pattern, &username, &password, true) {
                return Ok(Reply::error(error.to_string()));
            }
        }
        let updated = inner.db.execute(
            "UPDATE tasks SET status='queued',error=NULL,auth_realm=NULL,error_kind=NULL,auth_proxy=0,retry_count=0,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status='auth_required'",
            [task_id],
        )?;
        if updated == 0 {
            return Ok(Reply::error("task is not waiting for a login"));
        }
        // The typed credentials serve this task even when they are not remembered.
        inner
            .task_logins
            .insert(task_id, xdm_http::Login::new(username, password));
        let mut reply = Reply::accepted(task_id);
        reply.logins = inner.credentials.summaries();
        drop(inner);
        self.wake.notify_one();
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os_actions::RecordOnly;
    use crate::schedule::tests::FakeClock;
    use crate::test_helpers::*;
    use std::sync::Arc;
    use xdm_ipc::Command;

    #[test]
    fn login_commands_round_trip_without_passwords() {
        let (service, _, _) = service(1000);
        assert!(service.command(Command::LoginList).unwrap().logins.is_empty());
        let saved = service
            .command(Command::LoginSave {
                pattern: "http://127.0.0.1:1/".into(),
                username: "alice".into(),
                password: "s3cret".into(),
                enabled: true,
            })
            .unwrap();
        assert_eq!(saved.logins.len(), 1);
        let summary = &saved.logins[0];
        assert_eq!(
            (
                summary.pattern.as_str(),
                summary.username.as_str(),
                summary.enabled
            ),
            ("http://127.0.0.1:1/", "alice", true)
        );
        // Replies never carry the password.
        assert!(!serde_json::to_string(&saved).unwrap().contains("s3cret"));
        assert!(
            !serde_json::to_string(&service.command(Command::LoginList).unwrap())
                .unwrap()
                .contains("s3cret")
        );
        // Invalid patterns and fields are rejected with no row added.
        assert!(
            !service
                .command(Command::LoginSave {
                    pattern: "ftp://host/".into(),
                    username: "bob".into(),
                    password: "pw".into(),
                    enabled: true,
                })
                .unwrap()
                .ok
        );
        assert!(
            !service
                .command(Command::LoginSave {
                    pattern: "http://host/".into(),
                    username: String::new(),
                    password: "pw".into(),
                    enabled: true,
                })
                .unwrap()
                .ok
        );
        assert!(service.command(Command::LoginList).unwrap().logins.len() == 1);
        assert!(
            service
                .command(Command::LoginRemove { id: 999 })
                .unwrap()
                .error
                .is_some()
        );
        assert!(
            service
                .command(Command::LoginRemove { id: summary.id })
                .unwrap()
                .ok
        );
        assert!(service.command(Command::LoginList).unwrap().logins.is_empty());
    }

    #[test]
    fn credentials_file_lives_next_to_the_database() {
        let directory = std::env::temp_dir().join(format!("xdm-auth-db-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("tasks.sqlite3");
        {
            let file_service =
                Service::open_db(&database, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                    .unwrap();
            assert!(
                file_service
                    .command(Command::LoginSave {
                        pattern: "https://files.example.com/".into(),
                        username: "alice".into(),
                        password: "s3cret".into(),
                        enabled: true,
                    })
                    .unwrap()
                    .ok
            );
        }
        let store = directory.join("credentials.json");
        let stored = std::fs::read_to_string(&store).unwrap();
        assert!(
            stored.contains("s3cret"),
            "the store is the only place a password appears"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&store).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let reopened =
            Service::open_db(&database, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                .unwrap();
        let logins = reopened.command(Command::LoginList).unwrap().logins;
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].pattern, "https://files.example.com/");
        assert!(!serde_json::to_string(&logins).unwrap().contains("s3cret"));
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
