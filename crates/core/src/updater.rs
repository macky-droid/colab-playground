//! Installing an update in place when the user asks for it: download this computer's release
//! file, check it, put it where the running copy is, and start it again.
//!
//! The order is the safety. The signed checksum list is fetched and checked before the large
//! download; the download is checked before anything installed is touched; and only the last
//! step, a rename ([`replace`]), changes what is installed. Release files are fetched from URLs
//! built here from the repository and the tag, never from a reply.

pub mod download;
pub mod layout;
pub mod replace;
pub mod verify;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use layout::{Install, Platform, SelfUpdate, Unsupported};

use crate::clock::CancelToken;
use crate::error::{Error, ErrorKind, Result};
use crate::host::{Host, tail};
use crate::http::HttpClient;
use crate::update::{Release, Version};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum UpdateProgress {
    /// Fetching and checking the release's signed checksum list.
    Preparing,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    /// Checking the download (on macOS also unpacking it and checking the app's signature).
    Verifying,
    /// Putting the new version in place; no longer cancellable.
    Installing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub version: String,
    /// What to start to run the new version; `None` when nothing was installed (demo data).
    pub relaunch: Option<Relaunch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// What macOS needs to unpack a bundle and check it, kept apart so the rest runs anywhere.
pub trait MacTools {
    /// Unpacks the zip `archive` into `into`, keeping everything the code signature covers.
    fn unpack(&self, archive: &Path, into: &Path) -> Result<()>;
    /// Fails unless `app`'s code signature is intact.
    fn check_signature(&self, app: &Path) -> Result<()>;
}

/// [`MacTools`] with the system's `ditto` (the release workflow zips with it) and `codesign`,
/// run through [`Host`].
pub struct SystemMacTools<'a> {
    pub host: &'a Host,
    pub cancel: &'a CancelToken,
}

const MAC_TOOL_TIMEOUT: Duration = Duration::from_secs(300);

impl SystemMacTools<'_> {
    fn run(&self, program: &str, args: Vec<String>) -> Result<crate::host::ScriptOutput> {
        self.host
            .run_program(program, &args, &[], self.cancel, Some(MAC_TOOL_TIMEOUT))
    }
}

impl MacTools for SystemMacTools<'_> {
    fn unpack(&self, archive: &Path, into: &Path) -> Result<()> {
        let args = vec![
            "-x".into(),
            "-k".into(),
            path_arg(archive)?,
            path_arg(into)?,
        ];
        let out = self.run("/usr/bin/ditto", args)?;
        if out.success() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::Io).detail(format!("ditto: {}", tail(&out.stderr, 300))))
        }
    }

    fn check_signature(&self, app: &Path) -> Result<()> {
        let args = vec!["--verify".into(), "--strict".into(), path_arg(app)?];
        let out = self.run("/usr/bin/codesign", args)?;
        if out.success() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::UpdateUnverified)
                .detail(format!("codesign: {}", tail(&out.stderr, 300))))
        }
    }
}

fn path_arg(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        Error::new(ErrorKind::Io)
            .with("path", path.display())
            .detail("the path is not valid UTF-8")
    })
}

/// Where release `tag`'s files are. Only a `v<version>` tag is accepted, so the URL cannot be
/// steered anywhere else.
pub fn download_base(owner: &str, repo: &str, tag: &str) -> Result<String> {
    let valid = tag.starts_with('v')
        && Version::parse(tag).is_some()
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if !valid {
        return Err(Error::new(ErrorKind::Parse)
            .with("tag", tag)
            .detail("the release tag is not v<version>"));
    }
    Ok(format!(
        "https://github.com/{owner}/{repo}/releases/download/{tag}/"
    ))
}

/// The arguments this process started with, for the version that replaces it. (Old macOS
/// launchers add a process serial number, which is not ours to pass on.)
pub fn restart_args() -> Vec<String> {
    std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .filter(|arg| !arg.starts_with("-psn_"))
        .collect()
}

/// Everything one update needs.
pub struct Update<'a> {
    pub http: &'a dyn HttpClient,
    pub mac: &'a dyn MacTools,
    pub install: Install,
    pub platform: Platform,
    /// Target triple of this computer's release download.
    pub target: &'a str,
    pub owner: &'a str,
    pub repo: &'a str,
    pub release: Release,
    pub keys: Vec<verify::PublicKey>,
    /// What the new version is started with.
    pub args: Vec<String>,
}

impl Update<'_> {
    /// Downloads, checks and installs the release, reporting each phase on `events`. Cancel
    /// works until [`UpdateProgress::Installing`]. On any error the installed version is the
    /// old one and nothing of the attempt is left, except where a failed undo had to keep the
    /// old version in the staging folder (the error's `previous` names it).
    pub fn run(&self, events: &Sender<UpdateProgress>, cancel: &CancelToken) -> Result<Installed> {
        let send = |progress| {
            let _ = events.send(progress);
        };
        send(UpdateProgress::Preparing);
        let tag = &self.release.tag;
        let base = download_base(self.owner, self.repo, tag)?;
        let missing = |file: &str| {
            Error::new(ErrorKind::UpdateUnverified)
                .with("file", file)
                .detail("the release does not have it")
        };
        let sums = download::fetch_small(self.http, &format!("{base}SHA256SUMS"), cancel)?
            .ok_or_else(|| missing("SHA256SUMS"))?;
        let signature = download::fetch_small(self.http, &format!("{base}SHA256SUMS.sig"), cancel)?
            .ok_or_else(|| missing("SHA256SUMS.sig"))?;
        verify::check_signature(&verify::signed_message(tag, &sums), &signature, &self.keys)?;
        let name = layout::asset_name(self.target, self.platform);
        let expected = verify::listed_hash(&sums, &name)?.ok_or_else(|| {
            Error::new(ErrorKind::UpdateUnavailable)
                .with("file", &name)
                .detail("SHA256SUMS does not list it")
        })?;
        let folder = self
            .install
            .folder()
            .ok_or_else(|| Error::new(ErrorKind::Io).detail("the app is not in a folder"))?;
        let stage = replace::create_staging(folder)?;
        match self.install_from(&base, &name, &expected, &stage, &send, cancel) {
            Ok(()) => {
                // On Linux the old program was replaced by the rename; on Windows and macOS
                // it now sits in the staging folder, deleted on the next start.
                if matches!(self.install, Install::Binary { .. }) {
                    let _ = std::fs::remove_dir_all(&stage);
                }
                Ok(Installed {
                    version: self.release.version.clone(),
                    relaunch: Some(self.relaunch()),
                })
            }
            Err(error) => {
                if error.param("previous").is_none() {
                    let _ = std::fs::remove_dir_all(&stage);
                }
                Err(error)
            }
        }
    }

    fn install_from(
        &self,
        base: &str,
        name: &str,
        expected: &verify::Sha256,
        stage: &Path,
        send: &dyn Fn(UpdateProgress),
        cancel: &CancelToken,
    ) -> Result<()> {
        let download = stage.join(name);
        let io = |error: std::io::Error| Error::from(error).with("path", download.display());
        let mut file = File::create(&download).map_err(io)?;
        let actual = download::fetch_to(
            self.http,
            &format!("{base}{name}"),
            &mut file,
            download::MAX_DOWNLOAD,
            &mut |received, total| send(UpdateProgress::Downloading { received, total }),
            cancel,
        )?;
        file.sync_all().map_err(io)?;
        drop(file);
        send(UpdateProgress::Verifying);
        verify::check_hash(&actual, expected)?;
        let root = layout::archive_root(self.target);
        let new = match &self.install {
            Install::Exe { .. } => download,
            Install::Binary { .. } => replace::unpack_linux(&download, &root, stage)?,
            Install::Bundle { .. } => {
                let unpacked = stage.join("unpacked");
                self.mac.unpack(&download, &unpacked)?;
                let app = replace::find_bundle(&unpacked, &root)?;
                self.mac.check_signature(&app)?;
                app
            }
        };
        // The last moment a cancel applies: from here on the swap finishes or is undone.
        cancel.check()?;
        send(UpdateProgress::Installing);
        match &self.install {
            Install::Exe { path } => replace::swap_exe(path, &new, stage),
            Install::Binary { path } => replace::swap_binary(path, &new),
            Install::Bundle { app } => replace::swap_bundle(app, &new, stage),
        }
    }

    fn relaunch(&self) -> Relaunch {
        match &self.install {
            Install::Exe { path } | Install::Binary { path } => Relaunch {
                program: path.clone(),
                args: self.args.clone(),
            },
            // Through Launch Services, as a new instance: `open` would otherwise bring the
            // running (old) one to the front.
            Install::Bundle { app } => {
                let mut args = vec!["-n".to_owned(), app.display().to_string()];
                if !self.args.is_empty() {
                    args.push("--args".to_owned());
                    args.extend(self.args.iter().cloned());
                }
                Relaunch {
                    program: PathBuf::from("/usr/bin/open"),
                    args,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;

    use ed25519_dalek::SigningKey;

    use super::download::tests::Files;
    use super::replace::tests::linux_archive;
    use super::verify::tests::{key_pair, sign, sums_line};
    use super::*;

    const BASE: &str = "https://github.com/owner/repo/releases/download/v0.3.0/";

    fn release() -> Release {
        Release {
            version: "0.3.0".into(),
            tag: "v0.3.0".into(),
            page: "https://github.com/owner/repo/releases/tag/v0.3.0".into(),
        }
    }

    /// Unpacks a "zip" whose bytes are the new program into the layout the workflow zips.
    struct FakeMac {
        signature_ok: bool,
    }

    impl MacTools for FakeMac {
        fn unpack(&self, archive: &Path, into: &Path) -> Result<()> {
            let app = into
                .join("colab-playground-aarch64-apple-darwin")
                .join("Colab Playground.app");
            fs::create_dir_all(app.join("Contents/MacOS"))?;
            fs::write(app.join("Contents/Info.plist"), b"<plist/>")?;
            fs::copy(archive, app.join("Contents/MacOS/colab-playground"))?;
            Ok(())
        }

        fn check_signature(&self, _: &Path) -> Result<()> {
            if self.signature_ok {
                Ok(())
            } else {
                Err(Error::new(ErrorKind::UpdateUnverified).detail("codesign: invalid"))
            }
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        install: Install,
        platform: Platform,
        target: &'static str,
        key: SigningKey,
        public: verify::PublicKey,
    }

    impl Fixture {
        fn new(platform: Platform) -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let folder = dir.path().join("apps");
            fs::create_dir(&folder).unwrap();
            let (install, target) = match platform {
                Platform::Windows => {
                    let path = folder.join("colab-playground.exe");
                    fs::write(&path, b"old").unwrap();
                    (Install::Exe { path }, "x86_64-pc-windows-msvc")
                }
                Platform::Linux => {
                    let path = folder.join("colab-playground");
                    fs::write(&path, b"old").unwrap();
                    (Install::Binary { path }, "x86_64-unknown-linux-gnu")
                }
                Platform::MacOs => {
                    let app = folder.join("Colab Playground.app");
                    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
                    fs::write(app.join("Contents/MacOS/colab-playground"), b"old").unwrap();
                    (Install::Bundle { app }, "aarch64-apple-darwin")
                }
            };
            let (key, public) = key_pair();
            Fixture {
                dir,
                install,
                platform,
                target,
                key,
                public,
            }
        }

        fn asset(&self) -> String {
            layout::asset_name(self.target, self.platform)
        }

        /// The download as the release workflow makes it, holding program `program`.
        fn download(&self, program: &[u8]) -> Vec<u8> {
            match self.platform {
                Platform::Linux => linux_archive(&layout::archive_root(self.target), program, &[]),
                Platform::Windows | Platform::MacOs => program.to_vec(),
            }
        }

        /// A release as the workflow publishes it (signed for `tag`), serving `served` as the
        /// download while the list names `listed`.
        fn release_files(&self, tag: &str, listed: &[u8], served: &[u8]) -> Files {
            let sums = sums_line(listed, &self.asset());
            let signature = sign(&self.key, tag, sums.as_bytes());
            Files::default()
                .with(&format!("{BASE}SHA256SUMS"), sums.as_bytes())
                .with(&format!("{BASE}SHA256SUMS.sig"), &signature)
                .with(&format!("{BASE}{}", self.asset()), served)
        }

        fn good_files(&self) -> Files {
            let download = self.download(b"new");
            self.release_files("v0.3.0", &download, &download)
        }

        fn update<'a>(&'a self, http: &'a Files, mac: &'a FakeMac) -> Update<'a> {
            Update {
                http,
                mac,
                install: self.install.clone(),
                platform: self.platform,
                target: self.target,
                owner: "owner",
                repo: "repo",
                release: release(),
                keys: vec![self.public],
                args: vec!["--real".into()],
            }
        }

        fn program(&self) -> Vec<u8> {
            match &self.install {
                Install::Exe { path } | Install::Binary { path } => fs::read(path).unwrap(),
                Install::Bundle { app } => {
                    fs::read(app.join("Contents/MacOS/colab-playground")).unwrap()
                }
            }
        }

        /// What is in the folder besides the app: staging folders.
        fn staging(&self) -> Vec<PathBuf> {
            fs::read_dir(self.dir.path().join("apps"))
                .unwrap()
                .map(|e| e.unwrap())
                .filter(|e| replace::is_staging_name(&e.file_name().to_string_lossy()))
                .map(|e| e.path())
                .collect()
        }
    }

    fn run(update: &Update<'_>, cancel: &CancelToken) -> (Result<Installed>, Vec<UpdateProgress>) {
        let (events, received) = mpsc::channel();
        let result = update.run(&events, cancel);
        (result, received.try_iter().collect())
    }

    const OK_MAC: FakeMac = FakeMac { signature_ok: true };

    #[test]
    fn each_platform_installs_a_checked_release_in_place() {
        for platform in [Platform::Windows, Platform::Linux, Platform::MacOs] {
            let fixture = Fixture::new(platform);
            let http = fixture.good_files();
            let update = fixture.update(&http, &OK_MAC);
            let (result, events) = run(&update, &CancelToken::new());
            let installed = result.unwrap_or_else(|e| panic!("{platform:?}: {e}"));
            assert_eq!(fixture.program(), b"new", "{platform:?}");
            assert_eq!(installed.version, "0.3.0");
            assert_eq!(
                http.asked(),
                [
                    format!("{BASE}SHA256SUMS"),
                    format!("{BASE}SHA256SUMS.sig"),
                    format!("{BASE}{}", fixture.asset())
                ],
                "the signed list first, then the download"
            );
            assert_eq!(events.first(), Some(&UpdateProgress::Preparing));
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, UpdateProgress::Downloading { .. }))
            );
            assert_eq!(
                &events[events.len() - 2..],
                [UpdateProgress::Verifying, UpdateProgress::Installing]
            );
            let relaunch = installed.relaunch.unwrap();
            match &fixture.install {
                Install::Exe { path } | Install::Binary { path } => {
                    assert_eq!(&relaunch.program, path);
                    assert_eq!(relaunch.args, ["--real"]);
                }
                Install::Bundle { app } => {
                    assert_eq!(relaunch.program, Path::new("/usr/bin/open"));
                    assert_eq!(
                        relaunch.args,
                        ["-n", &app.display().to_string(), "--args", "--real"]
                    );
                }
            }
            // The replaced version waits for the next start only where it may still run.
            let left = fixture.staging().len();
            assert_eq!(
                left,
                usize::from(platform != Platform::Linux),
                "{platform:?}"
            );
            assert_eq!(
                replace::remove_leftovers(&fixture.dir.path().join("apps")),
                0
            );
        }
    }

    fn assert_untouched(fixture: &Fixture, result: Result<Installed>, kind: ErrorKind) {
        let error = result.unwrap_err();
        assert_eq!(error.kind, kind, "{error}");
        assert_eq!(fixture.program(), b"old", "the old version stays");
        assert!(
            fixture.staging().is_empty(),
            "nothing of the attempt is left"
        );
    }

    #[test]
    fn a_release_signed_with_another_key_is_never_downloaded() {
        for platform in [Platform::Windows, Platform::Linux, Platform::MacOs] {
            let fixture = Fixture::new(platform);
            let http = fixture.good_files();
            let mut update = fixture.update(&http, &OK_MAC);
            update.keys = vec![key_pair().1];
            let (result, _) = run(&update, &CancelToken::new());
            assert_untouched(&fixture, result, ErrorKind::UpdateUnverified);
            assert_eq!(http.asked().len(), 2, "only the list and its signature");
        }
    }

    #[test]
    fn an_older_signed_release_under_a_newer_tag_is_refused() {
        let fixture = Fixture::new(Platform::Windows);
        let download = fixture.download(b"old release");
        let http = fixture.release_files("v0.2.0", &download, &download);
        let (result, _) = run(&fixture.update(&http, &OK_MAC), &CancelToken::new());
        assert_untouched(&fixture, result, ErrorKind::UpdateUnverified);
    }

    #[test]
    fn a_tampered_download_is_not_installed() {
        for platform in [Platform::Windows, Platform::Linux, Platform::MacOs] {
            let fixture = Fixture::new(platform);
            let listed = fixture.download(b"new");
            let served = fixture.download(b"tampered");
            let http = fixture.release_files("v0.3.0", &listed, &served);
            let (result, _) = run(&fixture.update(&http, &OK_MAC), &CancelToken::new());
            assert_untouched(&fixture, result, ErrorKind::UpdateUnverified);
        }
    }

    #[test]
    fn a_release_without_a_signature_is_unverified() {
        let fixture = Fixture::new(Platform::Linux);
        let mut http = fixture.good_files();
        http.files.remove(&format!("{BASE}SHA256SUMS.sig"));
        let (result, _) = run(&fixture.update(&http, &OK_MAC), &CancelToken::new());
        assert_untouched(&fixture, result, ErrorKind::UpdateUnverified);
    }

    #[test]
    fn a_release_without_this_computers_download_is_unavailable() {
        let fixture = Fixture::new(Platform::Windows);
        let sums = sums_line(b"x", "colab-playground-other-target.exe");
        let signature = sign(&fixture.key, "v0.3.0", sums.as_bytes());
        let http = Files::default()
            .with(&format!("{BASE}SHA256SUMS"), sums.as_bytes())
            .with(&format!("{BASE}SHA256SUMS.sig"), &signature);
        let (result, _) = run(&fixture.update(&http, &OK_MAC), &CancelToken::new());
        assert_untouched(&fixture, result, ErrorKind::UpdateUnavailable);
    }

    #[test]
    fn a_cancelled_download_leaves_nothing() {
        let fixture = Fixture::new(Platform::Windows);
        let token = CancelToken::new();
        let mut http = fixture.release_files("v0.3.0", &[9u8; 20_000], &[9u8; 20_000]);
        http.cancel_after = Some((5_000, token.clone()));
        let (result, events) = run(&fixture.update(&http, &OK_MAC), &token);
        assert_untouched(&fixture, result, ErrorKind::Cancelled);
        assert!(!events.contains(&UpdateProgress::Installing));
    }

    #[test]
    fn a_macos_bundle_whose_signature_breaks_is_not_installed() {
        let fixture = Fixture::new(Platform::MacOs);
        let http = fixture.good_files();
        let bad = FakeMac {
            signature_ok: false,
        };
        let (result, _) = run(&fixture.update(&http, &bad), &CancelToken::new());
        assert_untouched(&fixture, result, ErrorKind::UpdateUnverified);
    }

    #[test]
    fn only_v_version_tags_make_download_urls() {
        assert_eq!(
            download_base("o", "r", "v1.2.3").unwrap(),
            "https://github.com/o/r/releases/download/v1.2.3/"
        );
        assert!(download_base("o", "r", "v1.2.3-rc.1").is_ok());
        for tag in [
            "1.2.3",
            "v1.2",
            "v1.2.3/../../x",
            "v1.2.3+build",
            "v1.2.3?x",
            "",
        ] {
            assert!(download_base("o", "r", tag).is_err(), "{tag}");
        }
    }
}
