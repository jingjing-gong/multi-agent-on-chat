//! Instance identity and compatibility checks run before Store opens for writing.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
pub const FORMAT_VERSION: i64 = 1;
pub fn identity() -> Value {
    json!({"package_version":env!("CARGO_PKG_VERSION"),"commit":env!("MAC_BUILD_COMMIT"),
        "dirty":env!("MAC_BUILD_DIRTY")=="true","release":env!("MAC_BUILD_RELEASE_ID"),
        "executable":std::env::current_exe().ok(),"instance_id":std::env::var("MAC_INSTANCE_ID").ok(),
        "environment":std::env::var("MAC_ENV").ok(),"format_version":FORMAT_VERSION,"database_schema_max":3,"control_protocol":1})
}
pub fn install_root() -> Option<PathBuf> {
    std::env::var_os("MAC_INSTALL_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".local/share/mac")))
}
pub fn preflight(root: &Path) -> Result<()> {
    if let Ok(bound) = std::env::var("MAC_BOUND_DATA_DIR") {
        anyhow::ensure!(
            root.canonicalize()? == Path::new(&bound).canonicalize()?,
            "data directory differs from selected instance"
        );
    }
    let marker = root.join("instance.json");
    if marker.exists() {
        let v: Value = serde_json::from_slice(&std::fs::read(marker)?)?;
        anyhow::ensure!(
            v["format_version"].as_i64() == Some(FORMAT_VERSION),
            "unsupported instance format; refusing database writes"
        );
        anyhow::ensure!(
            std::env::var("MAC_INSTANCE_ID").ok().as_deref() == v["instance_id"].as_str(),
            "managed instance must be opened through its mac-cli environment"
        );
        anyhow::ensure!(
            std::env::var("MAC_ENV").ok().as_deref() == v["name"].as_str(),
            "instance environment mismatch"
        );
    }
    let db = root.join("runtime.sqlite3");
    if !db.exists() {
        return Ok(());
    }
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    anyhow::ensure!(
        version <= 3,
        "database format {version} is newer than supported 3; refusing writes"
    );
    if root.join("instance.json").exists() {
        anyhow::ensure!(
            version == 3,
            "managed instance requires schema 3; explicit migration required"
        );
        use fs2::FileExt;
        let lock_path = root.join("service.lock");
        if lock_path.exists() {
            let file = std::fs::OpenOptions::new().write(true).open(lock_path)?;
            if file.try_lock_exclusive().is_err() {
                let raw: String = c.query_row(
                    "SELECT data FROM records WHERE kind='control' AND id='daemon'",
                    [],
                    |r| r.get(0),
                )?;
                let daemon: Value = serde_json::from_str(&raw)?;
                anyhow::ensure!(
                    daemon["identity"]["release"] == identity()["release"],
                    "active daemon version differs; refusing database writes"
                );
            }
        }
    }
    // Format 0 is the legacy compatible schema. Never reset a newer marker.
    Ok(())
}
fn canonical_reserved(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .context("workspace has no resolvable parent")?;
    Ok(canonical_reserved(parent)?.join(path.file_name().context("workspace filename missing")?))
}
pub fn workspace_check(root: &Path, workspace: &Path) -> Result<()> {
    if !root.join("instance.json").exists() {
        return Ok(());
    }
    let workspace = workspace.canonicalize()?;
    let Some(install) = install_root() else {
        return Ok(());
    };
    let entries = std::fs::read_dir(install.join("instances"))
        .context("managed instance registry missing")?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let v: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let other = Path::new(
            v["data_dir"]
                .as_str()
                .context("instance data directory missing")?,
        )
        .canonicalize()?;
        if other == root.canonicalize()? {
            continue;
        }
        let db = other.join("runtime.sqlite3");
        if !db.exists() {
            continue;
        }
        let c =
            rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut stmt = c.prepare("SELECT data FROM records WHERE kind IN ('agent','session')")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows {
            let record: Value = serde_json::from_str(&row?)?;
            if let Some(p) = record["workspace"].as_str() {
                let p = canonical_reserved(Path::new(p))?;
                if workspace.starts_with(&p) || p.starts_with(&workspace) {
                    bail!("workspace overlaps another instance: {}", p.display());
                }
            }
        }
    }
    Ok(())
}
pub fn bot_lock(bot: &crate::domain::Bot) -> Result<Option<std::fs::File>> {
    use fs2::FileExt;
    use sha2::{Digest, Sha256};
    if bot.channel == "local" {
        return Ok(None);
    }
    anyhow::ensure!(
        !bot.account_id.trim().is_empty(),
        "Bot platform identity missing"
    );
    let root = install_root()
        .context("install root unavailable")?
        .join("locks");
    std::fs::create_dir_all(&root)?;
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{}:{}", bot.channel, bot.account_id).as_bytes())
    );
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(format!("bot-{key}.lock")))?;
    file.try_lock_exclusive()
        .context("Bot platform identity is already listened to by another instance")?;
    Ok(Some(file))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_platform_identity_cannot_be_listened_twice() {
        let bot = crate::domain::Bot {
            id: "one".into(),
            channel: "lark".into(),
            account_id: format!("cli_{}", uuid::Uuid::new_v4()),
            credential_ref: None,
            enabled: true,
            owner: None,
        };
        let first = bot_lock(&bot).unwrap();
        let mut alias = bot.clone();
        alias.id = "other-local-name".into();
        assert!(bot_lock(&alias).is_err());
        drop(first);
        assert!(bot_lock(&alias).is_ok());
    }
    #[test]
    fn newer_database_is_rejected_without_mutation() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("runtime.sqlite3");
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch("PRAGMA user_version=99").unwrap();
        drop(c);
        assert!(preflight(d.path()).is_err());
        let c = rusqlite::Connection::open(&db).unwrap();
        assert_eq!(
            c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            99
        );
    }
}
