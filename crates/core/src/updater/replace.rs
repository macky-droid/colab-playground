//! Putting a checked download in place of the running copy.
//!
//! The download is unpacked into a staging folder next to what it replaces, on the same file
//! system, so the step that changes the installed files is a rename. Everything that can fail
//! for other reasons happens before it, and a failed swap is undone, so a failure leaves the old
//! version in place and runnable. The functions take paths only: each platform's strategy is
//! compiled and tested on every operating system.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{Error, ErrorKind, Result};

/// Start of a staging folder's name; the rest is 16 hex digits.
pub const STAGING_PREFIX: &str = ".colab-playground-update-";

/// The name of the program inside the Linux archive's folder.
pub const LINUX_PROGRAM: &str = "colab-playground";

/// Files beside the Linux program that an update refreshes where they already exist.
const LINUX_DOCS: [&str; 2] = ["LICENSE", "README.md"];

fn io_error(error: io::Error, path: &Path) -> Error {
    Error::from(error).with("path", path.display())
}

fn bad_archive(detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::Parse).detail(detail)
}

/// A new, empty staging folder in `folder`.
pub fn create_staging(folder: &Path) -> Result<PathBuf> {
    let stage = folder.join(format!("{STAGING_PREFIX}{}", crate::accounts::nonce()));
    fs::create_dir(&stage).map_err(|e| io_error(e, &stage))?;
    Ok(stage)
}

pub fn is_staging_name(name: &str) -> bool {
    name.strip_prefix(STAGING_PREFIX)
        .is_some_and(|rest| rest.len() == 16 && rest.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Removes the staging folders earlier updates left in `folder`, the replaced version among
/// them, and nothing else. Returns how many are still there: on Windows a program that is still
/// exiting cannot be deleted yet.
pub fn remove_leftovers(folder: &Path) -> usize {
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    let mut left = 0;
    for entry in entries.flatten() {
        let ours = entry.file_name().to_str().is_some_and(is_staging_name);
        // `file_type` does not follow links: a link with that name is not ours to empty.
        let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if ours && folder && fs::remove_dir_all(entry.path()).is_err() {
            left += 1;
        }
    }
    left
}

/// Moves `target` to `aside`, then `new` to where `target` was. When the second move fails the
/// first is undone; if even that fails, the error names where the old version is
/// (`previous`), and the caller must keep it.
fn swap_aside(target: &Path, new: &Path, aside: &Path) -> Result<()> {
    fs::rename(target, aside).map_err(|e| io_error(e, target))?;
    if let Err(error) = fs::rename(new, target) {
        let error = io_error(error, target);
        return Err(match fs::rename(aside, target) {
            Ok(()) => error,
            Err(_) => error.with("previous", aside.display()),
        });
    }
    Ok(())
}

/// Windows: a running program cannot be overwritten or deleted, but it can be renamed. Moves
/// the running `exe` into `stage` and `new` to where it was; the old file is deleted on the
/// next start ([`remove_leftovers`]).
pub fn swap_exe(exe: &Path, new: &Path, stage: &Path) -> Result<()> {
    swap_aside(exe, new, &stage.join("previous.exe"))
}

/// Linux: unpacks the program, and the documents beside it, from the release archive into
/// `stage`; returns the program, ready to run. Only those exact entries are read, so no entry
/// can write anywhere else.
pub fn unpack_linux(archive: &Path, root: &str, stage: &Path) -> Result<PathBuf> {
    let file = File::open(archive).map_err(|e| io_error(e, archive))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let out = stage.join("unpacked");
    fs::create_dir(&out).map_err(|e| io_error(e, &out))?;
    let wanted: Vec<(String, &str)> = std::iter::once(LINUX_PROGRAM)
        .chain(LINUX_DOCS)
        .map(|name| (format!("{root}/{name}"), name))
        .collect();
    let mut found = false;
    let entries = tar.entries().map_err(|e| bad_archive(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| bad_archive(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| bad_archive(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let path = path.trim_start_matches("./");
        let Some((_, name)) = wanted.iter().find(|(entry_path, _)| entry_path == path) else {
            continue;
        };
        if entry.header().entry_type() != tar::EntryType::Regular {
            return Err(bad_archive(format!("{path} is not a regular file")));
        }
        let dest = out.join(name);
        let mut file = File::create(&dest).map_err(|e| io_error(e, &dest))?;
        io::copy(&mut entry, &mut file).map_err(|e| io_error(e, &dest))?;
        file.sync_all().map_err(|e| io_error(e, &dest))?;
        found |= *name == LINUX_PROGRAM;
    }
    if !found {
        return Err(bad_archive(format!(
            "the archive has no {root}/{LINUX_PROGRAM}"
        )));
    }
    let program = out.join(LINUX_PROGRAM);
    make_executable(&program)?;
    Ok(program)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| io_error(e, path))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// Linux: renames `new` over `binary`. A rename within a folder is atomic: the path always
/// names a whole program, old or new, and the running process keeps its own file. Then the
/// documents unpacked beside `new` replace the ones beside `binary`, only where those exist (a
/// program moved out of its folder gets no files added next to it). A document that cannot be
/// replaced keeps its old text: the program is what matters.
pub fn swap_binary(binary: &Path, new: &Path) -> Result<()> {
    fs::rename(new, binary).map_err(|e| io_error(e, binary))?;
    if let (Some(from), Some(to)) = (new.parent(), binary.parent()) {
        for doc in LINUX_DOCS {
            let (src, dest) = (from.join(doc), to.join(doc));
            if src.is_file() && dest.is_file() {
                let _ = fs::rename(&src, &dest);
            }
        }
    }
    Ok(())
}

/// macOS: the bundle that unpacking the release archive into `unpacked` produced, the one
/// `.app` in `<root>`.
pub fn find_bundle(unpacked: &Path, root: &str) -> Result<PathBuf> {
    let folder = unpacked.join(root);
    let apps: Vec<PathBuf> = fs::read_dir(&folder)
        .map_err(|e| io_error(e, &folder))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
        .collect();
    match apps.as_slice() {
        [app] => Ok(app.clone()),
        _ => Err(bad_archive(format!(
            "expected one .app in {root}, found {}",
            apps.len()
        ))),
    }
}

/// macOS: moves the running bundle `app` into `stage` and `new` to where it was. The old
/// bundle is deleted on the next start ([`remove_leftovers`]), not while it still runs.
pub fn swap_bundle(app: &Path, new: &Path, stage: &Path) -> Result<()> {
    swap_aside(app, new, &stage.join("previous.app"))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write as _;

    use super::*;

    /// A release archive as the workflow builds it, plus `extra` entries.
    pub(crate) fn linux_archive(root: &str, program: &[u8], extra: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut add = |path: &str, body: &[u8], mode: u32| {
            let mut header = tar::Header::new_gnu();
            // Written raw: the builder refuses a `..` path, which a hostile archive could hold.
            header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_size(body.len() as u64);
            header.set_mode(mode);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append(&header, body).unwrap();
        };
        add(&format!("{root}/{LINUX_PROGRAM}"), program, 0o755);
        add(&format!("{root}/LICENSE"), b"new licence", 0o644);
        add(&format!("{root}/README.md"), b"new readme", 0o644);
        for (path, body) in extra {
            add(path, body, 0o644);
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn staging_folders_are_recognised_by_their_exact_name() {
        let dir = tempfile::tempdir().unwrap();
        let stage = create_staging(dir.path()).unwrap();
        let name = stage.file_name().unwrap().to_str().unwrap();
        assert!(is_staging_name(name), "{name}");
        for other in [
            ".colab-playground-update-",
            ".colab-playground-update-xyz",
            ".colab-playground-update-0123456789abcdef0",
            "colab-playground-update-0123456789abcdef",
        ] {
            assert!(!is_staging_name(other), "{other}");
        }
    }

    #[test]
    fn leftovers_are_removed_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let stage = create_staging(dir.path()).unwrap();
        fs::write(stage.join("previous.exe"), b"old").unwrap();
        fs::create_dir(dir.path().join(".colab-playground-update-notmine")).unwrap();
        fs::write(dir.path().join("colab-playground.exe"), b"program").unwrap();
        fs::write(
            dir.path().join(".colab-playground-update-0123456789abcdef"),
            b"a file with that name",
        )
        .unwrap();
        assert_eq!(remove_leftovers(dir.path()), 0);
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ".colab-playground-update-0123456789abcdef",
                ".colab-playground-update-notmine",
                "colab-playground.exe"
            ]
        );
        assert_eq!(remove_leftovers(&dir.path().join("missing")), 0);
    }

    #[test]
    fn the_windows_swap_moves_the_program_aside() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Colab Playground (1).exe");
        fs::write(&exe, b"old").unwrap();
        let stage = create_staging(dir.path()).unwrap();
        let new = stage.join("download.exe");
        fs::write(&new, b"new").unwrap();
        swap_exe(&exe, &new, &stage).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new", "same name, new program");
        assert_eq!(fs::read(stage.join("previous.exe")).unwrap(), b"old");
        assert_eq!(remove_leftovers(dir.path()), 0);
        assert!(!stage.exists());
    }

    #[test]
    fn a_failed_swap_puts_the_old_version_back() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("colab-playground.exe");
        fs::write(&exe, b"old").unwrap();
        let stage = create_staging(dir.path()).unwrap();
        let error = swap_exe(&exe, &stage.join("missing.exe"), &stage).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Io);
        assert_eq!(error.param("previous"), None, "undone");
        assert_eq!(fs::read(&exe).unwrap(), b"old");

        let app = dir.path().join("Colab Playground.app");
        fs::create_dir_all(app.join("Contents")).unwrap();
        swap_bundle(&app, &stage.join("missing.app"), &stage).unwrap_err();
        assert!(app.join("Contents").is_dir(), "the bundle is back");
    }

    #[test]
    fn the_linux_program_is_unpacked_executable_and_renamed_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let root = "colab-playground-x86_64-unknown-linux-gnu";
        let folder = dir.path().join("my apps");
        fs::create_dir(&folder).unwrap();
        let binary = folder.join(LINUX_PROGRAM);
        fs::write(&binary, b"old").unwrap();
        fs::write(folder.join("README.md"), b"old readme").unwrap();
        let archive = dir.path().join("download.tar.gz");
        // An entry outside the folder is never written.
        let escape: &[u8] = b"escaped";
        fs::write(
            &archive,
            linux_archive(
                root,
                b"new",
                &[("../escaped", escape), ("other/file", escape)],
            ),
        )
        .unwrap();
        let stage = create_staging(&folder).unwrap();
        let new = unpack_linux(&archive, root, &stage).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&new).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        swap_binary(&binary, &new).unwrap();
        assert_eq!(fs::read(&binary).unwrap(), b"new");
        assert_eq!(fs::read(folder.join("README.md")).unwrap(), b"new readme");
        assert!(
            !folder.join("LICENSE").exists(),
            "no document is added where there was none"
        );
        assert!(!dir.path().join("escaped").exists());
        assert!(!stage.join("other").exists());
    }

    #[test]
    fn an_archive_without_the_program_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("download.tar.gz");
        fs::write(&archive, linux_archive("another-root", b"new", &[])).unwrap();
        let stage = create_staging(dir.path()).unwrap();
        let error = unpack_linux(
            &archive,
            "colab-playground-x86_64-unknown-linux-gnu",
            &stage,
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Parse);
        let mut garbage = File::create(&archive).unwrap();
        garbage.write_all(b"not gzip").unwrap();
        drop(garbage);
        let stage = create_staging(dir.path()).unwrap();
        assert!(unpack_linux(&archive, "x", &stage).is_err());
    }

    #[test]
    fn the_macos_bundle_is_found_and_swapped_whole() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Colab Playground.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/MacOS/colab-playground"), b"old").unwrap();
        let stage = create_staging(dir.path()).unwrap();
        let root = "colab-playground-aarch64-apple-darwin";
        let unpacked = stage.join("unpacked");
        let new_app = unpacked.join(root).join("Colab Playground.app");
        fs::create_dir_all(new_app.join("Contents/MacOS")).unwrap();
        fs::write(new_app.join("Contents/MacOS/colab-playground"), b"new").unwrap();
        fs::write(unpacked.join(root).join("LICENSE"), b"licence").unwrap();
        let found = find_bundle(&unpacked, root).unwrap();
        assert_eq!(found, new_app);
        swap_bundle(&app, &found, &stage).unwrap();
        assert_eq!(
            fs::read(app.join("Contents/MacOS/colab-playground")).unwrap(),
            b"new"
        );
        assert!(stage.join("previous.app/Contents/MacOS").is_dir());
        assert!(find_bundle(&unpacked, "missing").is_err());
    }
}
