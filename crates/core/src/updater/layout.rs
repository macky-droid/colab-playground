//! Where the running copy is, what an update replaces, and why a copy cannot update itself.
//!
//! Every platform's rules are ordinary functions over paths, compiled and tested on every
//! operating system; which one applies comes from [`crate::host`], which owns what differs per
//! operating system.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Set by the release workflow when it builds a download. Without it the binary was built some
/// other way (`cargo run`, a local release build, a distribution's package), and replacing it
/// with a download would replace something the user or a package manager put there.
pub const RELEASE_BUILD: Option<&str> = option_env!("COLAB_PLAYGROUND_RELEASE_BUILD");

pub use crate::host::{Platform, release_target};

/// The release file for `target`, as the release workflow names it.
pub fn asset_name(target: &str, platform: Platform) -> String {
    let extension = match platform {
        Platform::Windows => ".exe",
        Platform::Linux => ".tar.gz",
        Platform::MacOs => ".zip",
    };
    format!("colab-playground-{target}{extension}")
}

/// The folder at the top of the Linux and macOS archives.
pub fn archive_root(target: &str) -> String {
    format!("colab-playground-{target}")
}

/// What an update replaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Install {
    /// Windows: the program file, which is the whole app.
    Exe { path: PathBuf },
    /// Linux: the program in the folder the archive unpacked.
    Binary { path: PathBuf },
    /// macOS: the whole `.app` bundle (its signature seals everything inside).
    Bundle { app: PathBuf },
}

impl Install {
    /// The folder that holds what is replaced; the download is unpacked next to it, on the
    /// same file system, so the final step is a rename.
    pub fn folder(&self) -> Option<&Path> {
        match self {
            Install::Exe { path } | Install::Binary { path } => path.parent(),
            Install::Bundle { app } => app.parent(),
        }
    }
}

/// Why this copy cannot replace itself. The app then offers the download page instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Unsupported {
    /// Not built by the release workflow (see [`RELEASE_BUILD`]).
    DevelopmentBuild,
    /// No release download is built for this system and processor.
    NoDownload,
    /// The folder holding the app cannot be written by this user.
    NotWritable { folder: String },
    /// macOS runs the app from a read-only copy because it was opened where it was unzipped
    /// (App Translocation). Moving it to Applications ends that.
    Translocated,
    /// macOS: the program does not run from inside its `.app` bundle.
    NotInBundle,
    /// The files around the program are not the ones a release ships.
    UnexpectedLayout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SelfUpdate {
    Supported,
    Unsupported(Unsupported),
}

/// What an update would replace for the program at `exe` on `platform`. `writable` says
/// whether a folder accepts new files (see [`probe_writable`]).
pub fn locate(
    platform: Platform,
    exe: &Path,
    writable: &dyn Fn(&Path) -> bool,
) -> Result<Install, Unsupported> {
    let not_writable = |folder: &Path| Unsupported::NotWritable {
        folder: folder.display().to_string(),
    };
    match platform {
        Platform::Windows | Platform::Linux => {
            if !exe.is_file() {
                return Err(Unsupported::UnexpectedLayout);
            }
            let folder = exe.parent().ok_or(Unsupported::UnexpectedLayout)?;
            if !writable(folder) {
                return Err(not_writable(folder));
            }
            let path = exe.to_path_buf();
            Ok(if platform == Platform::Windows {
                Install::Exe { path }
            } else {
                Install::Binary { path }
            })
        }
        Platform::MacOs => {
            if exe
                .components()
                .any(|part| part.as_os_str() == "AppTranslocation")
            {
                return Err(Unsupported::Translocated);
            }
            let named = |path: Option<&Path>, name: &str| {
                path.and_then(Path::file_name).is_some_and(|n| n == name)
            };
            let macos = exe.parent();
            let contents = macos.and_then(Path::parent);
            let app = contents.and_then(Path::parent);
            let is_app = app
                .and_then(Path::extension)
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
            let (true, true, true, Some(contents), Some(app)) = (
                named(macos, "MacOS"),
                named(contents, "Contents"),
                is_app,
                contents,
                app,
            ) else {
                return Err(Unsupported::NotInBundle);
            };
            if !exe.is_file() || !contents.join("Info.plist").is_file() {
                return Err(Unsupported::UnexpectedLayout);
            }
            let folder = app.parent().ok_or(Unsupported::UnexpectedLayout)?;
            if !writable(folder) {
                return Err(not_writable(folder));
            }
            Ok(Install::Bundle {
                app: app.to_path_buf(),
            })
        }
    }
}

/// Whether this process can create files in `folder`: tries it with a file it removes again
/// (permission bits and ACLs do not answer that reliably on every system).
pub fn probe_writable(folder: &Path) -> bool {
    let probe = folder.join(format!(
        ".colab-playground-write-{}",
        crate::accounts::nonce()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// What an update would replace for this running copy, and the target of its download.
pub fn this_copy() -> Result<(Install, Platform, &'static str), Unsupported> {
    if cfg!(debug_assertions) || RELEASE_BUILD.is_none_or(str::is_empty) {
        return Err(Unsupported::DevelopmentBuild);
    }
    let (Some(platform), Some(target)) = (Platform::current(), release_target()) else {
        return Err(Unsupported::NoDownload);
    };
    let exe = crate::host::current_program().map_err(|_| Unsupported::UnexpectedLayout)?;
    locate(platform, &exe, &probe_writable).map(|install| (install, platform, target))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn always(_: &Path) -> bool {
        true
    }

    fn never(_: &Path) -> bool {
        false
    }

    #[test]
    fn asset_names_follow_the_release_workflow() {
        assert_eq!(
            asset_name("x86_64-pc-windows-msvc", Platform::Windows),
            "colab-playground-x86_64-pc-windows-msvc.exe"
        );
        assert_eq!(
            asset_name("x86_64-unknown-linux-gnu", Platform::Linux),
            "colab-playground-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            asset_name("aarch64-apple-darwin", Platform::MacOs),
            "colab-playground-aarch64-apple-darwin.zip"
        );
        assert_eq!(
            archive_root("aarch64-apple-darwin"),
            "colab-playground-aarch64-apple-darwin"
        );
    }

    #[test]
    fn windows_and_linux_replace_the_program_file() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("colab-playground.exe");
        fs::write(&exe, b"program").unwrap();
        assert_eq!(
            locate(Platform::Windows, &exe, &always),
            Ok(Install::Exe { path: exe.clone() })
        );
        assert_eq!(
            locate(Platform::Linux, &exe, &always),
            Ok(Install::Binary { path: exe.clone() })
        );
        assert_eq!(
            locate(Platform::Windows, &exe, &never),
            Err(Unsupported::NotWritable {
                folder: dir.path().display().to_string()
            })
        );
        assert_eq!(
            locate(Platform::Linux, &dir.path().join("missing"), &always),
            Err(Unsupported::UnexpectedLayout)
        );
        assert_eq!(
            Install::Exe { path: exe }.folder(),
            Some(dir.path()),
            "unpacked next to it"
        );
    }

    fn bundle(root: &Path, name: &str) -> PathBuf {
        let app = root.join(name);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/Info.plist"), b"<plist/>").unwrap();
        let exe = app.join("Contents/MacOS/colab-playground");
        fs::write(&exe, b"program").unwrap();
        exe
    }

    #[test]
    fn macos_replaces_the_whole_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let exe = bundle(dir.path(), "Colab Playground.app");
        assert_eq!(
            locate(Platform::MacOs, &exe, &always),
            Ok(Install::Bundle {
                app: dir.path().join("Colab Playground.app")
            })
        );
        assert_eq!(
            locate(Platform::MacOs, &exe, &never),
            Err(Unsupported::NotWritable {
                folder: dir.path().display().to_string()
            })
        );
        // A renamed bundle is still a bundle.
        let renamed = bundle(dir.path(), "Colab.APP");
        assert!(locate(Platform::MacOs, &renamed, &always).is_ok());
    }

    #[test]
    fn macos_outside_a_bundle_or_translocated_cannot_update() {
        let dir = tempfile::tempdir().unwrap();
        let loose = dir.path().join("colab-playground");
        fs::write(&loose, b"program").unwrap();
        assert_eq!(
            locate(Platform::MacOs, &loose, &always),
            Err(Unsupported::NotInBundle)
        );
        let translocated = bundle(
            &dir.path().join("AppTranslocation/0A1B/d"),
            "Colab Playground.app",
        );
        assert_eq!(
            locate(Platform::MacOs, &translocated, &always),
            Err(Unsupported::Translocated)
        );
        let exe = bundle(dir.path(), "Colab Playground.app");
        fs::remove_file(dir.path().join("Colab Playground.app/Contents/Info.plist")).unwrap();
        assert_eq!(
            locate(Platform::MacOs, &exe, &always),
            Err(Unsupported::UnexpectedLayout)
        );
    }

    #[test]
    fn the_write_probe_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe_writable(dir.path()));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(!probe_writable(&dir.path().join("missing")));
    }

    #[test]
    fn a_test_build_is_a_development_build() {
        // Tests are never built by the release workflow.
        assert_eq!(this_copy().unwrap_err(), Unsupported::DevelopmentBuild);
    }
}
