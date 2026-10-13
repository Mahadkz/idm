use crate::{Result, Service};
use rusqlite::{Connection, params};
use std::io;
use url::Url;
use xdm_ipc::Reply;

/// User category rules persisted in the daemon database, sorted by name so
/// classification is deterministic.
pub(crate) fn load_rules(db: &Connection) -> Result<Vec<xdm_categories::CategoryRule>> {
    let mut statement =
        db.prepare("SELECT rule_json FROM categories ORDER BY name COLLATE NOCASE")?;
    Ok(statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .filter_map(|json| serde_json::from_str(json).ok())
        .collect())
}

/// Host of an http(s)/(f)tp URL, used by category site rules.
pub(crate) fn url_host(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .map(|host| {
            host.trim_start_matches('[')
                .trim_end_matches(']')
                .to_ascii_lowercase()
        })
}

/// Categories as they appear in the Categories dialog: edited built-ins first,
/// then the user's own categories.
pub(crate) fn category_reply(db: &Connection) -> Result<Reply> {
    let rules = load_rules(db)?;
    let mut categories = Vec::new();
    for builtin in xdm_categories::CATEGORIES {
        if let Some(rule) = rules.iter().find(|rule| rule.name == *builtin) {
            categories.push(rule.clone());
        } else {
            categories.push(xdm_categories::CategoryRule {
                name: builtin.to_string(),
                extensions: xdm_categories::builtin_extensions(builtin),
                destination: String::new(),
                sites: Vec::new(),
            });
        }
    }
    categories.extend(
        rules
            .into_iter()
            .filter(|rule| !xdm_categories::CATEGORIES.contains(&rule.name.as_str())),
    );
    let mut reply = Reply::list(Vec::new());
    reply.categories = categories;
    Ok(reply)
}

impl Service {
    pub(crate) fn category_save(
        &self,
        name: &str,
        extensions: Vec<String>,
        destination: String,
        sites: Vec<String>,
    ) -> Result<Reply> {
        let rule = xdm_categories::CategoryRule {
            name: xdm_categories::CATEGORIES
                .iter()
                .find(|builtin| builtin.eq_ignore_ascii_case(name))
                .map_or_else(|| name.to_string(), |builtin| builtin.to_string()),
            extensions,
            destination,
            sites,
        };
        if let Err(message) = xdm_categories::valid_rule(&rule) {
            return Ok(Reply::error(message));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        inner.db.execute(
            "INSERT INTO categories(name,rule_json) VALUES (?1,?2) ON CONFLICT(name) DO UPDATE SET rule_json=excluded.rule_json",
            params![name, serde_json::to_string(&rule)?],
        )?;
        category_reply(&inner.db)
    }

    pub(crate) fn category_remove(&self, name: &str) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if xdm_categories::CATEGORIES
            .iter()
            .any(|builtin| builtin.eq_ignore_ascii_case(name))
        {
            return Ok(Reply::error("built-in categories can be edited but not removed"));
        }
        if inner
            .db
            .execute("DELETE FROM categories WHERE name=?1", [name])?
            == 0
        {
            return Ok(Reply::error("category not found"));
        }
        category_reply(&inner.db)
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
    fn categories_list_save_and_remove_persist() {
        let (service, _, _) = service(1000);
        let listing = service.command(Command::CategoryList).unwrap();
        let names: Vec<String> = listing
            .categories
            .iter()
            .map(|rule| rule.name.clone())
            .collect();
        assert_eq!(
            names,
            xdm_categories::CATEGORIES
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        );
        service
            .command(Command::CategorySave {
                name: "Video".into(),
                extensions: vec!["mp4".into(), "webm".into()],
                destination: "/media".into(),
                sites: Vec::new(),
            })
            .unwrap();
        assert!(
            !service
                .command(Command::CategoryRemove {
                    name: "video".into()
                })
                .unwrap()
                .ok
        );
        let rule = |service: &Service, name: &str| {
            service
                .command(Command::CategoryList)
                .unwrap()
                .categories
                .into_iter()
                .find(|rule| rule.name == name)
                .unwrap()
        };
        assert_eq!(rule(&service, "Video").destination, "/media");
        // Rule edits survive a reopen.
        let path = std::env::temp_dir().join(format!("xdm-category-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let file_service =
                Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                    .unwrap();
            assert!(
                file_service
                    .command(Command::CategorySave {
                        name: "Docs".into(),
                        extensions: Vec::new(),
                        destination: String::new(),
                        sites: vec!["d.test".into()],
                    })
                    .unwrap()
                    .ok
            );
        }
        let reopened =
            Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                .unwrap();
        assert_eq!(rule(&reopened, "Docs").sites, vec!["d.test".to_string()]);
        assert!(
            reopened
                .command(Command::CategoryRemove {
                    name: "Docs".into()
                })
                .unwrap()
                .ok
        );
        assert!(
            !reopened
                .command(Command::CategoryRemove {
                    name: "Docs".into()
                })
                .unwrap()
                .ok
        );
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn submissions_classify_through_user_rules() {
        let (service, _, _) = service(1000);
        for (name, extensions, sites) in [
            ("Ebooks", vec!["epub".to_string()], Vec::new()),
            ("Publisher", Vec::new(), vec!["*.pub.test".to_string()]),
        ] {
            assert!(
                service
                    .command(Command::CategorySave {
                        name: name.into(),
                        extensions,
                        destination: String::new(),
                        sites,
                    })
                    .unwrap()
                    .ok
            );
        }
        let category = |request_id: &str, url: &str, output: &str| {
            let reply = service
                .command(Command::Submit {
                    request_id: request_id.into(),
                    url: url.into(),
                    output: output.into(),
                    headers: Vec::new(),
                    priority: 1,
                    connections: 1,
                })
                .unwrap();
            assert!(reply.ok, "{:?}", reply.error);
            service
                .command(Command::GetTask {
                    task_id: reply.task_id.unwrap(),
                })
                .unwrap()
                .tasks
                .remove(0)
                .category
        };
        assert_eq!(
            category(
                "rule-ext",
                "http://plain.test/x/letter.epub",
                "/nonexistent-xdm-test/letter.epub"
            ),
            "Ebooks"
        );
        // A host match beats an extension match on the same submission.
        assert_eq!(
            category(
                "rule-site",
                "http://www.pub.test/x.zip",
                "/nonexistent-xdm-test/marsh.zip"
            ),
            "Publisher"
        );
    }
}
