//! Where data lives.
//!
//! Two file systems are involved and are kept apart by type:
//!
//! * **App-side** files ([`PathBuf`]): `accounts.json`, the session ledger,
//!   installed plugins. Read and written directly by Rust.
//! * **Host-side** files ([`HostPath`]): the per-account CLI state files and
//!   scratch files, which the `colab` CLI (inside WSL on Windows) must read.
//!   They are only touched through [`crate::Host`] scripts.
//!
//! On Linux and macOS both live on the same disk, but the types still differ
//! so code written on one platform does the right thing on the other.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::accounts::AccountId;

/// A POSIX path on the host where the `colab` CLI runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostPath(pub String);

impl HostPath {
    pub fn new(p: impl Into<String>) -> Self {
        HostPath(p.into())
    }

    pub fn join(&self, seg: &str) -> HostPath {
        if self.0.ends_with('/') {
            HostPath(format!("{}{seg}", self.0))
        } else {
            HostPath(format!("{}/{seg}", self.0))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The parent directory (`/` for top-level paths).
    pub fn parent(&self) -> HostPath {
        match self.0.trim_end_matches('/').rsplit_once('/') {
            Some(("", _)) | None => HostPath("/".into()),
            Some((p, _)) => HostPath(p.to_owned()),
        }
    }
}

impl std::fmt::Display for HostPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    pub local: PathBuf,
    /// Host-side data directory (inside WSL on Windows).
    pub host: HostPath,
}

impl DataLayout {
    pub fn new(local: impl Into<PathBuf>, host: HostPath) -> Self {
        DataLayout {
            local: local.into(),
            host,
        }
    }

    /// `<local>/accounts.json`: non-secret account metadata.
    pub fn accounts_json(&self) -> PathBuf {
        self.local.join("accounts.json")
    }

    /// `<local>/ledger.json`: sessions the app provisioned.
    pub fn ledger_json(&self) -> PathBuf {
        self.local.join("ledger.json")
    }

    /// `<local>/plugins/`: user-installed plugins, one directory per id.
    pub fn plugins_dir(&self) -> PathBuf {
        self.local.join("plugins")
    }

    /// `<local>/plugins.json`: enabled/disabled flags.
    pub fn plugins_json(&self) -> PathBuf {
        self.local.join("plugins.json")
    }

    /// `<local>/staging/`: temporary plugin checkouts.
    pub fn staging_dir(&self) -> PathBuf {
        self.local.join("staging")
    }

    /// `<host>/accounts/<id>/sessions.json`: the account's `--config` state file.
    pub fn account_state(&self, id: &AccountId) -> HostPath {
        self.host
            .join("accounts")
            .join(id.as_str())
            .join("sessions.json")
    }

    /// `<host>/tmp`: scratch space for throw-away state files.
    pub fn host_scratch(&self) -> HostPath {
        self.host.join("tmp")
    }
}

/// The default app-side data directory (`<OS data dir>/colab-playground`).
/// Only used by the real app; tests always pass explicit directories.
pub fn default_local_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("colab-playground"))
}

/// Writes `bytes` to `path` atomically (temporary file + rename), creating
/// parent directories. Files are owner-only on unix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_paths_join_with_slashes() {
        let h = HostPath::new("/home/u/.local/share/colab-playground");
        let id = AccountId::new("a1b2").unwrap();
        let layout = DataLayout::new("C:\\data", h.clone());
        assert_eq!(
            layout.account_state(&id).as_str(),
            "/home/u/.local/share/colab-playground/accounts/a1b2/sessions.json"
        );
        assert_eq!(HostPath::new("/x/").join("y").as_str(), "/x/y");
        assert_eq!(
            layout.account_state(&id).parent().as_str(),
            "/home/u/.local/share/colab-playground/accounts/a1b2"
        );
        assert_eq!(HostPath::new("/x").parent().as_str(), "/");
    }

    #[test]
    fn atomic_write_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("f.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
    }
}
