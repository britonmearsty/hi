use crate::{config, providers::Message};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
pub static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn open() -> Result<Connection> {
    let path = config::data_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let db = Connection::open(path)?;
    migrate(&db)?;
    Ok(db)
}
fn migrate(db: &Connection) -> Result<()> {
    let version: i32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version == 0 {
        db.execute_batch("CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL, cwd TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT NOT NULL DEFAULT ''); CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, message TEXT NOT NULL, created_at INTEGER NOT NULL); PRAGMA user_version = 2;")?;
    } else if version == 1 {
        db.execute_batch("ALTER TABLE sessions ADD COLUMN summary TEXT NOT NULL DEFAULT ''; PRAGMA user_version = 2;")?;
    } else if version != 2 {
        anyhow::bail!("unsupported session database version: {version}");
    }
    Ok(())
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn create() -> Result<String> {
    let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    open()?.execute(
        "INSERT INTO sessions(id,title,cwd,created_at,updated_at,summary) VALUES (?1, ?2, ?3, ?4, ?4, '')",
        params![
            id,
            "New session",
            std::env::current_dir()?.display().to_string(),
            now()
        ],
    )?;
    Ok(id)
}
pub fn load(id: &str) -> Result<Vec<Message>> {
    let db = open()?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
        [id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("session not found: {id}");
    }
    let mut stmt = db.prepare("SELECT message FROM messages WHERE session_id=?1 ORDER BY id")?;
    let rows = stmt.query_map([id], |r| r.get::<_, String>(0))?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(serde_json::from_str(&row?)?);
    }
    Ok(messages)
}
pub fn save_message(id: &str, message: &Message) -> Result<()> {
    let db = open()?;
    db.execute(
        "INSERT INTO messages(session_id,message,created_at) VALUES (?1,?2,?3)",
        params![id, serde_json::to_string(message)?, now()],
    )?;
    db.execute(
        "UPDATE sessions SET updated_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    Ok(())
}
pub fn set_title(id: &str, title: &str) -> Result<()> {
    open()?.execute(
        "UPDATE sessions SET title=?2, updated_at=?3 WHERE id=?1",
        params![id, title.chars().take(80).collect::<String>(), now()],
    )?;
    Ok(())
}
pub fn clear_messages(id: &str) -> Result<()> {
    open()?.execute(
        "DELETE FROM messages WHERE session_id=?1 AND role != 'system'",
        [id],
    )?;
    Ok(())
}
pub fn get_summary(id: &str) -> Result<Option<String>> {
    let summary: String =
        open()?.query_row("SELECT summary FROM sessions WHERE id=?1", [id], |row| {
            row.get(0)
        })?;
    Ok((!summary.trim().is_empty()).then_some(summary))
}
pub fn set_summary(id: &str, summary: &str) -> Result<()> {
    open()?.execute(
        "UPDATE sessions SET summary=?2, updated_at=?3 WHERE id=?1",
        params![id, summary, now()],
    )?;
    Ok(())
}
pub fn list() -> Result<()> {
    let db = open()?;
    let mut stmt =
        db.prepare("SELECT id,title,updated_at FROM sessions ORDER BY updated_at DESC")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (id, title, updated) = row?;
        println!("{id}\t{updated}\t{title}");
    }
    Ok(())
}
pub fn delete(id: Option<String>, all: bool) -> Result<()> {
    let db = open()?;
    if all {
        db.execute("DELETE FROM messages", [])?;
        db.execute("DELETE FROM sessions", [])?;
        println!("Deleted all sessions.");
    } else if let Some(id) = id {
        let n = db.execute("DELETE FROM sessions WHERE id=?1", [&id])?;
        db.execute("DELETE FROM messages WHERE session_id=?1", [&id])?;
        if n == 0 {
            anyhow::bail!("session not found: {id}");
        }
        println!("Deleted {id}.");
    } else {
        anyhow::bail!("usage: hi delete <id> or hi delete --all");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_restores_messages() {
        let database = tempfile::NamedTempFile::new().unwrap();
        let connection = Connection::open(database.path()).unwrap();
        connection.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL, cwd TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL); CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, message TEXT NOT NULL, created_at INTEGER NOT NULL);").unwrap();
        connection
            .execute(
                "INSERT INTO sessions VALUES ('test', 'Test', '.', 0, 0)",
                [],
            )
            .unwrap();
        let original = Message {
            role: "user".into(),
            content: Some("hello".into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        };
        connection
            .execute(
                "INSERT INTO messages(session_id,message,created_at) VALUES ('test',?1,0)",
                [serde_json::to_string(&original).unwrap()],
            )
            .unwrap();
        let stored: String = connection
            .query_row(
                "SELECT message FROM messages WHERE session_id='test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let restored: Message = serde_json::from_str(&stored).unwrap();
        assert_eq!(restored.content, original.content);
    }

    #[test]
    fn public_session_lifecycle_works() {
        let _lock = TEST_LOCK.try_lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.path());
        let id = create().unwrap();
        let message = Message {
            role: "user".into(),
            content: Some("lifecycle".into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        };
        save_message(&id, &message).unwrap();
        assert_eq!(load(&id).unwrap()[0].content.as_deref(), Some("lifecycle"));
        delete(Some(id.clone()), false).unwrap();
        assert!(load(&id).is_err());
        std::env::remove_var("XDG_DATA_HOME");
    }
}
