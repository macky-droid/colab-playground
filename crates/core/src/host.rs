//! Where commands run.
//!
//! [`Host`] is the single seam through which this crate spawns processes. It
//! runs POSIX scripts either on the local machine ([`Host::Local`], unix only)
//! or inside WSL ([`Host::Wsl`], Windows only), plus direct program launches on
//! the local machine ([`Host::run_program`], used for `git`).
//!
//! # Script transport
//!
//! A script is never interpolated into a command line. Every invocation is
//!
//! ```text
//! bash -lc 'eval "$(printf %s "$0" | { base64 -d 2>/dev/null || base64 -D; })"' <b64 bootstrap> <b64 script> <job> <n-env> <b64 KEY=VALUE>... <b64 arg>...
//! ```
//!
//! The fixed bootstrap decodes the script, exports the environment, restores
//! the positional arguments and evaluates the script. Only the short fixed
//! `-lc` string and base64 text (`[A-Za-z0-9+/=]`) cross the Windows → WSL
//! command line, so quoting cannot corrupt anything. Large payloads (Python
//! helpers, plugin scripts) travel on stdin. Under WSL the environment is
//! passed positionally (the Windows process environment does not reach WSL);
//! locally it is set on the process so the login shell sees an overridden
//! `HOME` before reading profile files.
//!
//! # Process trees
//!
//! Scripts run as a background job with job control enabled, so they get their
//! own process group; the bootstrap reports its id on stderr. Killing a
//! [`HostChild`] (explicitly, on cancellation, or on drop) sends `SIGTERM` and
//! then `SIGKILL` to the whole group *on the host*, which also works for
//! processes inside WSL where killing `wsl.exe` alone would not.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};

use crate::clock::CancelToken;
use crate::error::{Error, ErrorKind, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Host {
    /// The local machine (POSIX shell). Script execution is unsupported on Windows.
    Local,
    /// A WSL distribution (Windows only). `None` means the default distribution.
    Wsl {
        /// Distribution name passed to `wsl.exe -d`.
        distro: Option<String>,
    },
}

/// The `bash -lc` string shared by every invocation; see the module docs.
/// (`base64 -D` is the spelling of older macOS releases.)
pub const BOOTSTRAP: &str = r#"eval "$(printf %s "$0" | { base64 -d 2>/dev/null || base64 -D; })""#;

/// Decoder run by [`BOOTSTRAP`]. Positional layout after `$0`:
/// `$1` script (b64), `$2` job flag, `$3` env count, then env pairs and args (b64).
const BOOT_BODY: &str = r#"__cpg_d() { printf '%s' "$1" | { base64 -d 2>/dev/null || base64 -D; } && printf x; }
__cpg_s=$(__cpg_d "$1") || exit 125
__cpg_s=${__cpg_s%x}
__cpg_j=$2
__cpg_n=$3
shift 3
while [ "$__cpg_n" -gt 0 ]; do
  __cpg_v=$(__cpg_d "$1") || exit 125
  export "${__cpg_v%x}"
  shift
  __cpg_n=$((__cpg_n - 1))
done
__cpg_a=()
for __cpg_x in "$@"; do
  __cpg_v=$(__cpg_d "$__cpg_x") || exit 125
  __cpg_a+=("${__cpg_v%x}")
done
set -- "${__cpg_a[@]}"
unset __cpg_a __cpg_x __cpg_v __cpg_n
unset -f __cpg_d
if [ "$__cpg_j" = 1 ]; then
  set -m
  ( eval "$__cpg_s" ) &
  __cpg_p=$!
  printf '\036CPG-PGID %s\n' "$__cpg_p" >&2
  wait "$__cpg_p"
  exit $?
fi
eval "$__cpg_s"
"#;

/// Kills a process group on the host: `$1` is the group id.
const KILL_SCRIPT: &str = r#"kill -TERM -- "-$1" 2>/dev/null || exit 0
i=0
while kill -0 -- "-$1" 2>/dev/null && [ "$i" -lt 30 ]; do sleep 0.1; i=$((i + 1)); done
kill -KILL -- "-$1" 2>/dev/null
exit 0
"#;

/// Prefix of the stderr line carrying the job's process group id.
const PGID_MARKER: &str = "\u{1e}CPG-PGID ";

/// Conservative Windows command-line budget (the hard limit is 32 767 UTF-16 units).
pub const MAX_WSL_COMMAND_LINE: usize = 30_000;

/// Windows `CREATE_NO_WINDOW` process creation flag.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StdinMode {
    /// `/dev/null` (the default). An unexpected prompt reads EOF instead of hanging.
    #[default]
    Null,
    /// These bytes, then EOF.
    Bytes(Vec<u8>),
    /// Kept open for [`HostChild::write_stdin`].
    Pipe,
}

/// A POSIX script plus everything it receives. Values are never interpolated
/// into the script text; read them from `"$1"`… or the environment.
#[derive(Debug, Clone, Default)]
pub struct ScriptRequest {
    /// Script source (bash).
    pub script: String,
    /// Positional arguments (`$1`…).
    pub args: Vec<String>,
    /// Extra environment variables. Keys must match `^[A-Za-z_][A-Za-z0-9_]*$`.
    pub env: Vec<(String, String)>,
    pub stdin: StdinMode,
    /// Kill the script after this long ([`Host::run`] returns `Timeout`).
    pub timeout: Option<Duration>,
}

impl ScriptRequest {
    pub fn new(script: impl Into<String>) -> Self {
        ScriptRequest {
            script: script.into(),
            ..Default::default()
        }
    }
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
    pub fn stdin_bytes(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = StdinMode::Bytes(data.into());
        self
    }
    pub fn stdin_pipe(mut self) -> Self {
        self.stdin = StdinMode::Pipe;
        self
    }
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One line of output (without the trailing newline, lossily UTF-8 decoded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub stream: Stream,
    /// Text without `\n` / `\r\n`.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineAction {
    Continue,
    /// Kill the process tree now (e.g. an unexpected interactive prompt).
    Kill,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptOutput {
    /// Exit code, `None` if terminated by a signal.
    pub code: Option<i32>,
    /// All stdout lines joined with `\n`.
    pub stdout: String,
    /// All stderr lines joined with `\n` (the internal job marker removed).
    pub stderr: String,
    /// True when a line callback requested a kill.
    pub killed: bool,
}

impl ScriptOutput {
    /// Exit code 0 and not killed.
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.killed
    }
}

/// A fully specified process launch, computed without spawning anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    /// Environment variables set on the spawned process itself.
    pub env: Vec<(String, String)>,
    /// Whether to pass `CREATE_NO_WINDOW` (Windows).
    pub no_window: bool,
}

pub fn is_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Quotes `s` for a POSIX shell (single quotes). Used only where a tool insists
/// on a shell string, such as OpenSSH's `ProxyCommand`.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@+,".contains(c))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Encodes the `bash` argument vector (everything after `bash`).
///
/// With `positional_env` the environment travels as base64 arguments decoded by
/// the bootstrap (needed for WSL); otherwise the caller sets it on the process.
pub(crate) fn encode_bash_args(
    req: &ScriptRequest,
    job: bool,
    positional_env: bool,
) -> Result<Vec<String>> {
    for (k, _) in &req.env {
        if !is_env_key(k) {
            return Err(Error::invalid(
                "env",
                format!("invalid environment variable name {k:?}"),
            ));
        }
    }
    let mut args = vec![
        "-lc".to_owned(),
        BOOTSTRAP.to_owned(),
        B64.encode(BOOT_BODY),
        B64.encode(&req.script),
        if job { "1" } else { "0" }.to_owned(),
    ];
    if positional_env {
        args.push(req.env.len().to_string());
        for (k, v) in &req.env {
            args.push(B64.encode(format!("{k}={v}")));
        }
    } else {
        args.push("0".to_owned());
    }
    args.extend(req.args.iter().map(|a| B64.encode(a)));
    Ok(args)
}

/// Approximate length of a Windows command line for `program` + `args`.
fn command_line_len(program: &str, args: &[String]) -> usize {
    program.len()
        + args
            .iter()
            .map(|a| a.len() + 3 + a.matches('"').count())
            .sum::<usize>()
}

impl Host {
    pub fn detect() -> Host {
        if cfg!(windows) {
            Host::Wsl { distro: None }
        } else {
            Host::Local
        }
    }

    /// Computes how a script would be launched, without spawning anything.
    ///
    /// `job` runs the script in its own process group (always used by
    /// [`Host::run`] and [`Host::spawn`]).
    pub fn invocation(&self, req: &ScriptRequest, job: bool) -> Result<Invocation> {
        match self {
            Host::Local => {
                if cfg!(windows) {
                    // `bash` on Windows usually resolves to System32\bash.exe, the
                    // WSL launcher. Never run it implicitly.
                    return Err(Error::new(ErrorKind::Unsupported).detail(
                        "POSIX scripts cannot run on a Windows Host::Local; use Host::Wsl",
                    ));
                }
                Ok(Invocation {
                    program: "bash".to_owned(),
                    args: encode_bash_args(req, job, false)?,
                    env: req.env.clone(),
                    no_window: false,
                })
            }
            Host::Wsl { distro } => {
                let mut args = Vec::new();
                if let Some(d) = distro {
                    if d.is_empty()
                        || d.starts_with('-')
                        || d.chars().any(|c| c.is_whitespace() || c == '"')
                    {
                        return Err(Error::invalid(
                            "distro",
                            format!("invalid WSL distribution name {d:?}"),
                        ));
                    }
                    args.push("-d".to_owned());
                    args.push(d.clone());
                }
                args.push("-e".to_owned());
                args.push("bash".to_owned());
                args.extend(encode_bash_args(req, job, true)?);
                let len = command_line_len("wsl.exe", &args);
                if len > MAX_WSL_COMMAND_LINE {
                    return Err(Error::new(ErrorKind::CommandTooLong)
                        .with("length", len)
                        .detail("send large payloads on stdin"));
                }
                Ok(Invocation {
                    program: "wsl.exe".to_owned(),
                    args,
                    env: vec![("WSL_UTF8".to_owned(), "1".to_owned())],
                    no_window: true,
                })
            }
        }
    }

    /// Runs a script to completion, streaming lines to `on_line`.
    ///
    /// Cancellation and `req.timeout` kill the whole process tree and return
    /// `Cancelled` / `Timeout`. A [`LineAction::Kill`] from the callback kills
    /// the tree and returns the output so far with `killed = true`.
    pub fn run(
        &self,
        req: ScriptRequest,
        cancel: &CancelToken,
        on_line: &mut dyn FnMut(&Line) -> LineAction,
    ) -> Result<ScriptOutput> {
        let timeout = req.timeout;
        let mut child = self.spawn(req)?;
        collect(&mut child, timeout, cancel, on_line)
    }

    pub fn run_quiet(&self, req: ScriptRequest, cancel: &CancelToken) -> Result<ScriptOutput> {
        self.run(req, cancel, &mut |_| LineAction::Continue)
    }

    /// The returned handle kills the process tree when dropped.
    pub fn spawn(&self, req: ScriptRequest) -> Result<HostChild> {
        if matches!(self, Host::Wsl { .. }) && !cfg!(windows) {
            return Err(
                Error::new(ErrorKind::Unsupported).detail("WSL is only available on Windows")
            );
        }
        let inv = self.invocation(&req, true)?;
        let (stdin_cfg, initial) = match req.stdin {
            StdinMode::Null => (Stdio::null(), None),
            StdinMode::Bytes(b) => (Stdio::piped(), Some(b)),
            StdinMode::Pipe => (Stdio::piped(), None),
        };
        let mut child = spawn_invocation(&inv, stdin_cfg).map_err(|e| match (self, e.kind()) {
            (Host::Wsl { .. }, std::io::ErrorKind::NotFound) => {
                Error::new(ErrorKind::WslUnavailable).detail(e.to_string())
            }
            (_, std::io::ErrorKind::NotFound) => {
                Error::new(ErrorKind::NotInstalled).with("program", &inv.program)
            }
            _ => Error::from(e),
        })?;
        let mut stdin = child.stdin.take();
        if let Some(bytes) = initial
            && let Some(mut pipe) = stdin.take()
        {
            thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            });
        }
        Ok(HostChild::start(self.clone(), child, stdin))
    }

    /// Runs `program` directly (no shell) on the local machine. Only valid for
    /// [`Host::Local`] (on any OS); used for `git clone` of plugins.
    pub fn run_program(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
        cancel: &CancelToken,
        timeout: Option<Duration>,
    ) -> Result<ScriptOutput> {
        if *self != Host::Local {
            return Err(Error::new(ErrorKind::Unsupported)
                .detail("run_program runs on the local machine only"));
        }
        let inv = Invocation {
            program: program.to_owned(),
            args: args.to_vec(),
            env: env.to_vec(),
            no_window: cfg!(windows),
        };
        let child = spawn_invocation(&inv, Stdio::null()).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                Error::new(ErrorKind::NotInstalled).with("program", program)
            }
            _ => Error::from(e),
        })?;
        let mut child = HostChild::start(Host::Local, child, None);
        child.direct = true;
        collect(&mut child, timeout, cancel, &mut |_| LineAction::Continue)
    }

    /// Reports what is available on this host. Spawns real processes; never
    /// call it from tests.
    pub fn probe(&self) -> ProbeReport {
        let mut report = ProbeReport::default();
        let cancel = CancelToken::new();
        if let Host::Wsl { .. } = self {
            if !cfg!(windows) {
                report.errors.push("wsl requires Windows".into());
                return report;
            }
            let inv = Invocation {
                program: "wsl.exe".into(),
                args: vec!["-l".into(), "-q".into()],
                env: vec![("WSL_UTF8".into(), "1".into())],
                no_window: true,
            };
            match spawn_invocation(&inv, Stdio::null()) {
                Ok(child) => {
                    let mut child = HostChild::start(Host::Local, child, None);
                    child.direct = true;
                    match collect(
                        &mut child,
                        Some(Duration::from_secs(30)),
                        &cancel,
                        &mut |_| LineAction::Continue,
                    ) {
                        Ok(out) => {
                            report.wsl_installed = true;
                            report.wsl_distros = out
                                .stdout
                                .lines()
                                .map(|l| {
                                    l.trim_matches(|c: char| c.is_whitespace() || c == '\0')
                                        .to_owned()
                                })
                                .filter(|l| !l.is_empty())
                                .collect();
                            if !out.success() {
                                report.errors.push(format!("wsl -l failed: {}", out.stderr));
                            }
                        }
                        Err(e) => report.errors.push(e.to_string()),
                    }
                }
                Err(e) => {
                    report.errors.push(format!("wsl.exe: {e}"));
                    return report;
                }
            }
            if report.wsl_distros.is_empty() {
                return report;
            }
        }
        let req = ScriptRequest::new(PROBE_SCRIPT).timeout(Duration::from_secs(60));
        match self.run_quiet(req, &cancel) {
            Ok(out) => report.apply_probe_output(&out.stdout),
            Err(e) => report.errors.push(e.to_string()),
        }
        report
    }
}

/// Probe script: prints `CPG key=value` lines.
const PROBE_SCRIPT: &str = r#"for t in bash ssh ssh-keygen git uv python3 colab; do
  p=$(command -v "$t" 2>/dev/null) && printf 'CPG %s=%s\n' "$t" "$p"
done
if c=$(command -v colab 2>/dev/null); then
  v=$("$c" version 2>/dev/null | head -n 1)
  printf 'CPG colab_version=%s\n' "${v#Version: }"
  r=$(readlink -f "$c" 2>/dev/null || printf %s "$c")
  l1=$(head -n 1 "$r" 2>/dev/null)
  l2=$(sed -n 2p "$r" 2>/dev/null)
  py=""
  case "$l1" in
    '#!/bin/sh'*|'#!/usr/bin/env sh'*) case "$l2" in *exec*) py="$(dirname "$r")/python" ;; esac ;;
    '#!'*) py=${l1#??}; set -- $py; py=$1; case "$py" in */env) py=$(command -v "$2") ;; esac ;;
  esac
  [ -n "$py" ] && printf 'CPG helper_python=%s\n' "$py"
  [ -n "$py" ] && "$py" -c 'import keyring' >/dev/null 2>&1 && printf 'CPG keyring=ok\n'
fi
[ -f "$HOME/.ssh/id_ed25519.pub" ] || [ -f "$HOME/.ssh/id_ecdsa.pub" ] && printf 'CPG ssh_key=present\n'
printf 'CPG home=%s\n' "$HOME"
"#;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeReport {
    /// `wsl.exe` answered (Windows only).
    pub wsl_installed: bool,
    /// Installed WSL distributions (default first, as `wsl -l -q` lists them).
    pub wsl_distros: Vec<String>,
    /// Paths of tools found on the host `PATH` (login shell).
    pub bash: Option<String>,
    pub ssh: Option<String>,
    pub ssh_keygen: Option<String>,
    /// `git` path (on the host; plugin clones use the local `git`).
    pub git: Option<String>,
    pub uv: Option<String>,
    pub python3: Option<String>,
    pub colab: Option<String>,
    pub colab_version: Option<String>,
    /// Interpreter of the `colab` entry script (runs the account helper).
    pub helper_python: Option<String>,
    /// The helper interpreter can `import keyring`.
    pub keyring_ok: bool,
    /// `~/.ssh/id_ed25519.pub` or `id_ecdsa.pub` exists.
    pub ssh_key_present: bool,
    pub home: Option<String>,
    /// Problems encountered while probing (log text).
    pub errors: Vec<String>,
}

impl ProbeReport {
    /// Parses `CPG key=value` lines produced by the probe script.
    pub fn apply_probe_output(&mut self, stdout: &str) {
        for line in stdout.lines() {
            let Some(rest) = line.strip_prefix("CPG ") else {
                continue;
            };
            let Some((k, v)) = rest.split_once('=') else {
                continue;
            };
            let v = v.trim().to_owned();
            let val = if v.is_empty() { None } else { Some(v) };
            match k {
                "bash" => self.bash = val,
                "ssh" => self.ssh = val,
                "ssh-keygen" => self.ssh_keygen = val,
                "git" => self.git = val,
                "uv" => self.uv = val,
                "python3" => self.python3 = val,
                "colab" => self.colab = val,
                "colab_version" => self.colab_version = val,
                "helper_python" => self.helper_python = val,
                "keyring" => self.keyring_ok = val.as_deref() == Some("ok"),
                "ssh_key" => self.ssh_key_present = val.as_deref() == Some("present"),
                "home" => self.home = val,
                _ => {}
            }
        }
    }

    /// The toolchain, when both paths were found.
    pub fn toolchain(&self) -> Option<Toolchain> {
        Some(Toolchain {
            colab: self.colab.clone()?,
            helper_python: self.helper_python.clone()?,
        })
    }
}

/// Paths (as seen by the host shell) of the `colab` executable and the Python
/// interpreter that runs it (and therefore has its `keyring` and `google-auth`).
///
/// An empty `helper_python` (see [`Toolchain::from_colab_path`]) means "derive
/// it from the shebang of `colab`"; the real backend does so on first use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toolchain {
    /// Absolute path of `colab`.
    pub colab: String,
    /// Absolute path of the CLI's Python interpreter (empty: derive it).
    pub helper_python: String,
}

/// Validates a `colab` path on the host and derives its interpreter. Prints
/// `CPG key=value` lines; `reason=` explains a failure.
const RESOLVE_SCRIPT: &str = r#"c=$1
[ -e "$c" ] || { echo "CPG reason=not_found"; exit 0; }
r=$(readlink -f -- "$c" 2>/dev/null || printf %s "$c")
[ -f "$r" ] || { echo "CPG reason=not_a_file"; exit 0; }
[ -x "$r" ] || { echo "CPG reason=not_executable"; exit 0; }
l1=$(head -n 1 -- "$r" 2>/dev/null)
l2=$(sed -n 2p -- "$r" 2>/dev/null)
py=""
case "$l1" in
  '#!/bin/sh'*|'#!/usr/bin/env sh'*) case "$l2" in *exec*) py="$(dirname -- "$r")/python" ;; esac ;;
  '#!'*) py=${l1#??}; set -- $py; py=$1; case "$py" in */env) py=$(command -v "$2" 2>/dev/null) ;; esac ;;
esac
case "$py" in *python*) ;; *) echo "CPG reason=no_python_shebang"; exit 0 ;; esac
[ -x "$py" ] || { printf 'CPG interpreter=%s
CPG reason=interpreter_missing
' "$py"; exit 0; }
printf 'CPG helper_python=%s
' "$py"
"#;

impl Toolchain {
    /// A toolchain from an overridden `colab` path only; the interpreter is
    /// derived from its shebang on first use ([`Toolchain::resolve`]).
    pub fn from_colab_path(colab: impl Into<String>) -> Toolchain {
        Toolchain {
            colab: colab.into(),
            helper_python: String::new(),
        }
    }

    pub fn needs_resolution(&self) -> bool {
        self.helper_python.is_empty()
    }

    /// Checks `colab` on the host (absolute, exists, executable, a Python
    /// entry script) and fills in `helper_python` from its shebang (also the
    /// `uv` / pip `#!/bin/sh` launcher form). Runs no Python. Spawns a host
    /// script: real mode only. Failures are `InvalidInput` with
    /// `field = "colab_path"`, `value`, and `reason`: `not_absolute`,
    /// `not_found`, `not_a_file`, `not_executable`, `no_python_shebang`, or
    /// `interpreter_missing` (with `interpreter`).
    pub fn resolve(
        &self,
        host: &Host,
        env: &[(String, String)],
        cancel: &CancelToken,
    ) -> Result<Toolchain> {
        if !self.needs_resolution() {
            return Ok(self.clone());
        }
        if !self.colab.starts_with('/') {
            return Err(colab_path_error(&self.colab, "not_absolute", None));
        }
        let mut req = ScriptRequest::new(RESOLVE_SCRIPT)
            .arg(&self.colab)
            .timeout(Duration::from_secs(60));
        req.env = env.to_vec();
        let out = host.run_quiet(req, cancel)?;
        parse_resolution(&self.colab, &out.stdout)
    }
}

fn colab_path_error(path: &str, reason: &str, interpreter: Option<&str>) -> Error {
    let what = match reason {
        "not_absolute" => "give the absolute path of the colab executable on the host",
        "not_found" => "no file at this path on the host; check the path (inside WSL on Windows)",
        "not_a_file" => "the path is not a regular file",
        "not_executable" => {
            "the file is not executable (chmod +x, or point at the installed entry script)"
        }
        "no_python_shebang" => {
            "not a Python entry script; point at the colab installed by uv or pip"
        }
        "interpreter_missing" => {
            "the interpreter named in its shebang does not exist; reinstall the CLI"
        }
        _ => "invalid colab path",
    };
    let mut e = Error::invalid("colab_path", what)
        .with("value", path)
        .with("reason", reason);
    if let Some(i) = interpreter {
        e = e.with("interpreter", i);
    }
    e
}

pub fn parse_resolution(colab: &str, stdout: &str) -> Result<Toolchain> {
    let mut helper = None;
    let mut reason = None;
    let mut interpreter = None;
    for line in stdout.lines() {
        let Some((k, v)) = line.strip_prefix("CPG ").and_then(|r| r.split_once('=')) else {
            continue;
        };
        match k {
            "helper_python" => helper = Some(v.trim().to_owned()),
            "reason" => reason = Some(v.trim().to_owned()),
            "interpreter" => interpreter = Some(v.trim().to_owned()),
            _ => {}
        }
    }
    match (helper, reason) {
        (Some(h), None) if !h.is_empty() => Ok(Toolchain {
            colab: colab.to_owned(),
            helper_python: h,
        }),
        (_, Some(r)) => Err(colab_path_error(colab, &r, interpreter.as_deref())),
        _ => Err(Error::new(ErrorKind::Parse).detail(tail(stdout, 300))),
    }
}

fn spawn_invocation(inv: &Invocation, stdin: Stdio) -> std::io::Result<Child> {
    let mut cmd = Command::new(&inv.program);
    cmd.args(&inv.args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in &inv.env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        if inv.no_window {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
    }
    cmd.spawn()
}

/// A running script. Dropping it kills the whole process tree.
pub struct HostChild {
    host: Host,
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Line>,
    pgid: Arc<Mutex<Option<u32>>>,
    exit: Option<Option<i32>>,
    direct: bool,
}

impl std::fmt::Debug for HostChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostChild")
            .field("host", &self.host)
            .field("pid", &self.child.id())
            .finish()
    }
}

impl HostChild {
    fn start(host: Host, mut child: Child, stdin: Option<ChildStdin>) -> HostChild {
        let (tx, rx) = mpsc::channel();
        let pgid = Arc::new(Mutex::new(None));
        if let Some(out) = child.stdout.take() {
            spawn_reader(out, Stream::Stdout, tx.clone(), pgid.clone());
        }
        if let Some(err) = child.stderr.take() {
            spawn_reader(err, Stream::Stderr, tx, pgid.clone());
        }
        HostChild {
            host,
            child,
            stdin,
            lines: rx,
            pgid,
            exit: None,
            direct: false,
        }
    }

    /// Waits up to `timeout` for the next output line. `Err(Disconnected)`
    /// means both streams are closed.
    pub fn recv_line(&self, timeout: Duration) -> std::result::Result<Line, RecvTimeoutError> {
        self.lines.recv_timeout(timeout)
    }

    /// Writes to stdin (only with [`StdinMode::Pipe`]).
    pub fn write_stdin(&mut self, data: &[u8]) -> Result<()> {
        match self.stdin.as_mut() {
            Some(pipe) => {
                pipe.write_all(data)?;
                pipe.flush()?;
                Ok(())
            }
            None => Err(Error::new(ErrorKind::Io).detail("stdin is not a pipe")),
        }
    }

    /// Closes stdin (EOF for the script).
    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// Returns the exit code if the process has exited (`Some(None)` = signal).
    pub fn try_wait(&mut self) -> Option<Option<i32>> {
        if self.exit.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.exit = Some(status.code());
        }
        self.exit
    }

    pub fn wait(&mut self) -> Option<i32> {
        if let Some(code) = self.exit {
            return code;
        }
        let code = self.child.wait().ok().and_then(|s| s.code());
        self.exit = Some(code);
        code
    }

    /// Kills the process tree (idempotent).
    pub fn kill(&mut self) {
        if self.try_wait().is_some() {
            return;
        }
        let pgid = *self.pgid.lock().unwrap_or_else(|e| e.into_inner());
        if let (false, Some(pgid)) = (self.direct, pgid) {
            let req = ScriptRequest::new(KILL_SCRIPT).arg(pgid.to_string());
            if let Ok(inv) = self.host.invocation(&req, false)
                && let Ok(mut killer) = spawn_invocation(&inv, Stdio::null())
            {
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    if let Ok(Some(_)) = killer.try_wait() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                let _ = killer.kill();
                let _ = killer.wait();
            }
        }
        let _ = self.child.kill();
        let code = self.child.wait().ok().and_then(|s| s.code());
        self.exit = Some(code);
    }
}

impl Drop for HostChild {
    fn drop(&mut self) {
        self.stdin = None;
        self.kill();
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    mut r: R,
    stream: Stream,
    tx: Sender<Line>,
    pgid: Arc<Mutex<Option<u32>>>,
) {
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();
        let emit = |bytes: &[u8]| -> bool {
            let mut text = String::from_utf8_lossy(bytes).into_owned();
            if text.ends_with('\r') {
                text.pop();
            }
            if stream == Stream::Stderr
                && let Some(id) = text.strip_prefix(PGID_MARKER)
            {
                if let Ok(id) = id.trim().parse::<u32>() {
                    *pgid.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
                }
                return true;
            }
            tx.send(Line { stream, text }).is_ok()
        };
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=pos).collect();
                        if !emit(&line[..line.len() - 1]) {
                            return;
                        }
                    }
                }
            }
        }
        if !pending.is_empty() {
            emit(&pending);
        }
    });
}

fn collect(
    child: &mut HostChild,
    timeout: Option<Duration>,
    cancel: &CancelToken,
    on_line: &mut dyn FnMut(&Line) -> LineAction,
) -> Result<ScriptOutput> {
    let start = Instant::now();
    let mut out = ScriptOutput::default();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut drain_deadline: Option<Instant> = None;
    loop {
        match child.recv_line(Duration::from_millis(50)) {
            Ok(line) => {
                let action = if out.killed {
                    LineAction::Continue
                } else {
                    on_line(&line)
                };
                match line.stream {
                    Stream::Stdout => stdout.push(line.text),
                    Stream::Stderr => stderr.push(line.text),
                }
                if action == LineAction::Kill {
                    out.killed = true;
                    child.kill();
                    drain_deadline = Some(Instant::now() + Duration::from_secs(2));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if let Some(d) = drain_deadline {
            if Instant::now() > d {
                break;
            }
            continue;
        }
        if cancel.is_cancelled() {
            child.kill();
            return Err(Error::cancelled());
        }
        if let Some(t) = timeout
            && start.elapsed() > t
        {
            child.kill();
            return Err(Error::new(ErrorKind::Timeout).detail(tail(&stderr.join("\n"), 400)));
        }
    }
    out.code = if out.killed {
        child.try_wait().flatten()
    } else {
        child.wait()
    };
    out.stdout = stdout.join("\n");
    out.stderr = stderr.join("\n");
    Ok(out)
}

/// Last `max` bytes of `s` (on a char boundary), for error details.
pub fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.trim().to_owned();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(s: &str) -> String {
        String::from_utf8(B64.decode(s).unwrap()).unwrap()
    }

    #[test]
    fn wsl_invocation_is_base64_only_after_the_bootstrap() {
        let host = Host::Wsl {
            distro: Some("Ubuntu-24.04".into()),
        };
        let req = ScriptRequest::new("printf '%s' \"$1\"")
            .arg("héllo \"world\" $(rm -rf /)")
            .env("COLAB_X", "a b\nc");
        let inv = host.invocation(&req, true).unwrap();
        assert_eq!(inv.program, "wsl.exe");
        assert!(inv.no_window);
        assert_eq!(inv.env, vec![("WSL_UTF8".to_owned(), "1".to_owned())]);
        assert_eq!(
            &inv.args[..6],
            &["-d", "Ubuntu-24.04", "-e", "bash", "-lc", BOOTSTRAP]
        );
        let rest = &inv.args[6..];
        let b64ish = |s: &str| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
        };
        assert!(rest.iter().all(|a| b64ish(a)), "{rest:?}");
        assert_eq!(decode(&rest[0]), BOOT_BODY);
        assert_eq!(decode(&rest[1]), req.script);
        assert_eq!(rest[2], "1");
        assert_eq!(rest[3], "1");
        assert_eq!(decode(&rest[4]), "COLAB_X=a b\nc");
        assert_eq!(decode(&rest[5]), "héllo \"world\" $(rm -rf /)");
    }

    #[test]
    fn default_distro_omits_dash_d() {
        let inv = Host::Wsl { distro: None }
            .invocation(&ScriptRequest::new("true"), false)
            .unwrap();
        assert_eq!(&inv.args[..3], &["-e", "bash", "-lc"]);
        assert_eq!(inv.args[6], "0");
    }

    #[test]
    fn bad_distro_and_env_key_are_rejected() {
        let host = Host::Wsl {
            distro: Some("-x".into()),
        };
        assert!(
            host.invocation(&ScriptRequest::new("true"), true)
                .unwrap_err()
                .is(ErrorKind::InvalidInput)
        );
        let host = Host::Wsl { distro: None };
        let req = ScriptRequest::new("true").env("BAD-KEY", "x");
        assert!(
            host.invocation(&req, true)
                .unwrap_err()
                .is(ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn oversized_wsl_command_line_is_refused() {
        let req = ScriptRequest::new("true").arg("x".repeat(40_000));
        let err = Host::Wsl { distro: None }
            .invocation(&req, true)
            .unwrap_err();
        assert!(err.is(ErrorKind::CommandTooLong));
    }

    #[cfg(unix)]
    #[test]
    fn local_invocation_sets_env_on_process() {
        let req = ScriptRequest::new("true").env("HOME", "/tmp/x").arg("a");
        let inv = Host::Local.invocation(&req, true).unwrap();
        assert_eq!(inv.program, "bash");
        assert_eq!(inv.env, vec![("HOME".to_owned(), "/tmp/x".to_owned())]);
        assert_eq!(inv.args[5], "0");
        assert_eq!(decode(&inv.args[6]), "a");
    }

    #[cfg(windows)]
    #[test]
    fn local_scripts_are_unsupported_on_windows() {
        let err = Host::Local
            .invocation(&ScriptRequest::new("true"), true)
            .unwrap_err();
        assert!(err.is(ErrorKind::Unsupported));
    }

    #[test]
    fn env_keys_and_quoting() {
        assert!(is_env_key("PLUGIN_INPUT_REPO"));
        assert!(is_env_key("_x1"));
        assert!(!is_env_key("1x"));
        assert!(!is_env_key("a-b"));
        assert!(!is_env_key(""));
        assert_eq!(shell_quote("/usr/bin/colab"), "/usr/bin/colab");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn probe_output_parsing() {
        let mut r = ProbeReport::default();
        r.apply_probe_output(
            "motd noise\nCPG colab=/home/u/.local/bin/colab\nCPG helper_python=/home/u/.local/share/uv/tools/google-colab-cli/bin/python\nCPG keyring=ok\nCPG colab_version=0.4.1\n",
        );
        assert_eq!(r.colab_version.as_deref(), Some("0.4.1"));
        assert!(r.keyring_ok);
        let tc = r.toolchain().unwrap();
        assert!(tc.helper_python.ends_with("/bin/python"));
    }

    #[test]
    fn colab_path_override_resolution() {
        let t = Toolchain::from_colab_path("/opt/colab");
        assert!(t.needs_resolution());
        let ok =
            parse_resolution("/opt/colab", "noise\nCPG helper_python=/venv/bin/python\n").unwrap();
        assert_eq!(ok.helper_python, "/venv/bin/python");
        assert!(!ok.needs_resolution());
        let e = parse_resolution(
            "/opt/colab",
            "CPG interpreter=/gone/python\nCPG reason=interpreter_missing\n",
        )
        .unwrap_err();
        assert!(e.is(ErrorKind::InvalidInput));
        assert_eq!(e.param("field"), Some("colab_path"));
        assert_eq!(e.param("reason"), Some("interpreter_missing"));
        assert_eq!(e.param("interpreter"), Some("/gone/python"));
        assert_eq!(
            parse_resolution("/x", "CPG reason=not_found")
                .unwrap_err()
                .param("reason"),
            Some("not_found")
        );
        assert!(
            parse_resolution("/x", "garbage")
                .unwrap_err()
                .is(ErrorKind::Parse)
        );
        // Relative paths are refused before anything runs.
        let e = Toolchain::from_colab_path("colab")
            .resolve(&Host::Wsl { distro: None }, &[], &CancelToken::new())
            .unwrap_err();
        assert_eq!(e.param("reason"), Some("not_absolute"));
        // An already complete toolchain resolves to itself without spawning.
        let full = Toolchain {
            colab: "/c".into(),
            helper_python: "/p".into(),
        };
        assert_eq!(
            full.resolve(&Host::Wsl { distro: None }, &[], &CancelToken::new())
                .unwrap(),
            full
        );
    }

    #[test]
    fn tail_respects_char_boundaries() {
        assert_eq!(tail("abc", 10), "abc");
        assert_eq!(tail("ééé", 3), "é");
    }

    /// Runs the exact WSL encoding (positional env) through a local bash.
    #[cfg(unix)]
    #[test]
    fn bootstrap_round_trips_args_env_stdin_and_exit_code() {
        let req = ScriptRequest::new(
            "printf 'a1=[%s] a2=[%s] n=%s\\n' \"$1\" \"$2\" \"$#\"; printf 'env=[%s]\\n' \"$CPG_T\"; cat; exit 7",
        )
        .arg("héllo 'q' \"d\" $(x) `y`")
        .arg("line1\nline2\n")
        .env("CPG_T", "v=1 ñ\n")
        .stdin_bytes("from stdin ✓\n");
        let args = encode_bash_args(&req, true, true).unwrap();
        let inv = Invocation {
            program: "bash".into(),
            args,
            env: vec![],
            no_window: false,
        };
        let child = spawn_invocation(&inv, Stdio::piped()).unwrap();
        let mut child = HostChild::start(Host::Local, child, None);
        let mut pipe = child.child.stdin.take().unwrap();
        pipe.write_all("from stdin ✓\n".as_bytes()).unwrap();
        drop(pipe);
        let out = collect(
            &mut child,
            Some(Duration::from_secs(20)),
            &CancelToken::new(),
            &mut |_| LineAction::Continue,
        )
        .unwrap();
        assert_eq!(out.code, Some(7));
        assert!(
            out.stdout.contains("a1=[héllo 'q' \"d\" $(x) `y`]"),
            "{}",
            out.stdout
        );
        assert!(
            out.stdout.contains("a2=[line1\nline2\n] n=2"),
            "{}",
            out.stdout
        );
        assert!(out.stdout.contains("env=[v=1 ñ\n]"), "{}", out.stdout);
        assert!(out.stdout.contains("from stdin ✓"));
        assert!(!out.stderr.contains("CPG-PGID"));
    }

    #[cfg(unix)]
    #[test]
    fn kill_takes_down_the_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived");
        let req = ScriptRequest::new("(sleep 3; touch \"$1\") & echo started; wait")
            .arg(marker.to_string_lossy().to_string())
            .stdin_pipe();
        let mut child = Host::Local.spawn(req).unwrap();
        let line = child.recv_line(Duration::from_secs(10)).unwrap();
        assert_eq!(line.text, "started");
        // Give the reader a moment to record the group id.
        std::thread::sleep(Duration::from_millis(200));
        child.kill();
        std::thread::sleep(Duration::from_secs(4));
        assert!(!marker.exists(), "background grandchild survived the kill");
    }

    #[cfg(unix)]
    #[test]
    fn line_callback_can_kill_and_cancel_works() {
        let req = ScriptRequest::new("echo prompt >&2; sleep 30");
        let out = Host::Local
            .run(req, &CancelToken::new(), &mut |l| {
                if l.text == "prompt" {
                    LineAction::Kill
                } else {
                    LineAction::Continue
                }
            })
            .unwrap();
        assert!(out.killed);
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.cancel();
        });
        let err = Host::Local
            .run_quiet(ScriptRequest::new("sleep 30"), &cancel)
            .unwrap_err();
        assert!(err.is(ErrorKind::Cancelled));
        let err = Host::Local
            .run_quiet(
                ScriptRequest::new("sleep 30").timeout(Duration::from_millis(300)),
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(err.is(ErrorKind::Timeout));
    }
}
