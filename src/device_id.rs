//! Stable, per-user receiver identity used by mDNS discovery.

use anyhow::{Context, Result};
use fs4::fs_std::FileExt;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub fn resolve(explicit: Option<&str>) -> Result<String> {
    if let Some(id) = explicit {
        return Ok(id.to_owned());
    }
    let path = default_path()?;
    load_or_create(&path).with_context(|| {
        format!(
            "could not load or save receiver identity at {}; use --device-id to set one explicitly",
            path.display()
        )
    })
}

fn absolute_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn default_path() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = absolute_env_path("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    #[cfg(target_os = "macos")]
    let base = absolute_env_path("HOME")
        .context("HOME is not set")?
        .join("Library/Application Support");
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = absolute_env_path("XDG_CONFIG_HOME")
        .or_else(|| absolute_env_path("HOME").map(|home| home.join(".config")))
        .context("neither an absolute XDG_CONFIG_HOME nor HOME is set")?;
    Ok(base.join("openchromecast").join("device-id"))
}

fn load_or_create(path: &Path) -> Result<String> {
    let parent = path
        .parent()
        .context("identity path has no parent directory")?;
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(parent)?;

    // Serialize first use so another process cannot read a partially written ID.
    let lock = private_file_options()
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock_exclusive()?;

    match fs::read_to_string(path) {
        Ok(saved) => Ok(Uuid::parse_str(saved.trim())
            .context("stored receiver identity is not a UUID")?
            .simple()
            .to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let id = Uuid::new_v4().simple().to_string();
            let mut file = private_file_options().create_new(true).open(path)?;
            writeln!(file, "{id}")?;
            file.sync_all()?;
            Ok(id)
        }
        Err(error) => Err(error.into()),
    }
}

fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Barrier};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("openchromecast-id-test-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn identity_path(&self) -> PathBuf {
            self.0.join("config").join("device-id")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn repeated_starts_reuse_the_same_identity() {
        let dir = TestDir::new();
        let first = load_or_create(&dir.identity_path()).unwrap();
        let second = load_or_create(&dir.identity_path()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn generated_identity_is_a_persisted_simple_uuid() {
        let dir = TestDir::new();
        let id = load_or_create(&dir.identity_path()).unwrap();
        assert_eq!(id.len(), 32);
        assert_eq!(Uuid::parse_str(&id).unwrap().get_version_num(), 4);
        assert_eq!(
            fs::read_to_string(dir.identity_path()).unwrap(),
            format!("{id}\n")
        );
    }

    #[test]
    fn explicit_identity_keeps_existing_cli_behavior() {
        assert_eq!(
            resolve(Some("Existing-CLI-Value")).unwrap(),
            "Existing-CLI-Value"
        );
    }

    #[test]
    fn separate_installations_do_not_share_an_identity() {
        let first = TestDir::new();
        let second = TestDir::new();
        assert_ne!(
            load_or_create(&first.identity_path()).unwrap(),
            load_or_create(&second.identity_path()).unwrap()
        );
    }

    #[test]
    fn existing_identity_is_read_without_rewriting() {
        let dir = TestDir::new();
        fs::create_dir_all(dir.identity_path().parent().unwrap()).unwrap();
        let saved = "d5e03bf8-0120-43a0-a752-f7d4d7c22c6b\n";
        fs::write(dir.identity_path(), saved).unwrap();
        assert_eq!(
            load_or_create(&dir.identity_path()).unwrap(),
            "d5e03bf8012043a0a752f7d4d7c22c6b"
        );
        assert_eq!(fs::read_to_string(dir.identity_path()).unwrap(), saved);
    }

    #[test]
    fn malformed_identity_is_not_silently_replaced() {
        let dir = TestDir::new();
        fs::create_dir_all(dir.identity_path().parent().unwrap()).unwrap();
        fs::write(dir.identity_path(), "not-a-uuid").unwrap();
        assert!(load_or_create(&dir.identity_path()).is_err());
        assert_eq!(
            fs::read_to_string(dir.identity_path()).unwrap(),
            "not-a-uuid"
        );
    }

    #[test]
    fn empty_identity_is_not_silently_replaced() {
        let dir = TestDir::new();
        fs::create_dir_all(dir.identity_path().parent().unwrap()).unwrap();
        fs::write(dir.identity_path(), "").unwrap();
        assert!(load_or_create(&dir.identity_path()).is_err());
        assert_eq!(fs::read_to_string(dir.identity_path()).unwrap(), "");
    }

    #[test]
    fn unavailable_storage_is_an_error_instead_of_an_ephemeral_id() {
        let dir = TestDir::new();
        fs::write(dir.0.join("config"), "existing file").unwrap();
        assert!(load_or_create(&dir.identity_path()).is_err());
        assert_eq!(
            fs::read_to_string(dir.0.join("config")).unwrap(),
            "existing file"
        );
    }

    #[test]
    fn concurrent_initialization_returns_one_identity() {
        let dir = TestDir::new();
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let barrier = barrier.clone();
                let path = dir.identity_path();
                std::thread::spawn(move || {
                    barrier.wait();
                    load_or_create(&path).unwrap()
                })
            })
            .collect();
        let ids: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert!(ids.iter().all(|id| id == &ids[0]));
    }

    #[cfg(unix)]
    #[test]
    fn identity_and_lock_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        load_or_create(&dir.identity_path()).unwrap();
        for path in [
            dir.identity_path(),
            dir.identity_path().with_extension("lock"),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
