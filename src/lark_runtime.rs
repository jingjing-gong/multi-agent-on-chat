//! Stable IPC namespace, independent of display names and registration attempts.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn config_path(credentials: &Path, app: &str) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME required for Lark runtime")?;
    config_path_at(credentials, app, &PathBuf::from(home).join(".mac-ipc"))
}
fn config_path_at(credentials: &Path, app: &str, root: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        !app.is_empty() && app.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
        "invalid Lark app identity"
    );
    let credentials = std::fs::canonicalize(credentials).context("resolve Lark credentials")?;
    let socket = credentials.join("events").join(app).join("bus.sock");
    if socket.as_os_str().len() < 104 {
        return Ok(credentials);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        std::fs::create_dir_all(root)?;
        anyhow::ensure!(
            std::fs::symlink_metadata(root)?.file_type().is_dir(),
            "Lark IPC root must be a directory"
        );
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
        let hash = format!(
            "{:x}",
            Sha256::digest(credentials.as_os_str().as_encoded_bytes())
        );
        let alias = root.join(&hash[..16]);
        anyhow::ensure!(
            alias
                .join("events")
                .join(app)
                .join("bus.sock")
                .as_os_str()
                .len()
                < 104,
            "Lark IPC root too long for Unix sockets"
        );
        match symlink(&credentials, &alias) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                anyhow::ensure!(
                    std::fs::symlink_metadata(&alias)?.file_type().is_symlink()
                        && std::fs::canonicalize(&alias)? == credentials,
                    "Lark IPC alias conflicts with existing path"
                );
            }
            Err(e) => return Err(e.into()),
        }
        Ok(alias)
    }
    #[cfg(not(unix))]
    {
        Ok(credentials)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_paths_get_stable_isolated_aliases_without_moving_credentials() {
        let root = tempfile::Builder::new()
            .prefix("mi")
            .tempdir_in("/tmp")
            .unwrap();
        let a = root.path().join("a".repeat(130));
        let b = root.path().join("b".repeat(130));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("config.json"), b"original").unwrap();
        let ipc = root.path().join("ipc");
        let alias = config_path_at(&a, "cli_test", &ipc).unwrap();
        assert_eq!(alias, config_path_at(&a, "cli_test", &ipc).unwrap());
        assert_ne!(alias, config_path_at(&b, "cli_test", &ipc).unwrap());
        assert_eq!(
            std::fs::read(alias.join("config.json")).unwrap(),
            b"original"
        );
        assert!(a.exists());
        std::fs::remove_file(&alias).unwrap();
        std::fs::create_dir(&alias).unwrap();
        assert!(config_path_at(&a, "cli_test", &ipc).is_err());
    }
}
