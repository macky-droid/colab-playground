# Colab Playground — Design

> A desktop app for the Google Colab CLI: accounts, GPU sessions with cost guards, port
> forwarding and plugins.

That sentence is the project description everywhere (`Cargo.toml`, README, About, GitHub repo);
change it in `[workspace.package]` and copy it, never paraphrase. Built with Rust + GPUI.
Serving a Hugging Face GGUF model with llama.cpp is the first bundled plugin. MIT, © ScarletKc.

This document is the single source of truth for implementers. Decisions here are settled; if
something turns out to be impossible, stop and report instead of silently diverging.

## 0. Product principle (the tie-breaker)

**Everything must match how a normal user expects it to work, and be logically sound.** When
this document is silent or two options compete, pick the one a first-time user would predict.
Concretely:

- Speak in the user's terms: "Deploy a model", "Stop and end billing", "Forward port" — never
  "run plugin llama-cpp" or CLI jargon. The plugin system is an implementation detail on the
  quick-deploy path.
- **Anything shown as running can be stopped from where it is shown**, on any saved account,
  including runtimes the app did not create. If an action is impossible, the control says why and
  offers the next best step; it is never silently missing or dead.
- Money is never ambiguous: closing a tunnel or the app does not stop billing, and the UI says
  so at that moment. Auto-stop warns ahead of time and offers "Extend".
- No required input the app can choose itself (session names are generated; forms are pre-filled
  from Defaults). Destructive or costly actions confirm once, stating the consequence.
- Long operations show what is happening now, what comes next, and can be cancelled; cancel
  always leaves no billed VM behind. Errors say what happened and what to do.
- State is honest: stale data is marked stale, estimates are marked `≈`, and mock mode carries a
  visible "Demo data" badge.
- Switching account switches what the app shows and acts on; before switching it lists what is
  affected (open tunnels close; sessions on the old account keep running under their watchdog).

## 1. Goals and non-goals (v1)

Goals

- Windows first (the Colab CLI runs inside WSL); macOS and Linux must build and behave the same.
  Platform-specific code is isolated behind `cfg` and one `Host` abstraction.
- UI built from [Ely GPUI Components](https://github.com/ELY-Open-Source/Ely-GPUI-Components).
  Never hand-roll a widget Ely already ships. Layout glue with `div()` is fine.
- English, 简体中文, 繁體中文. Light, dark and follow-system appearance.
- Colab core: CLI install, accounts (login / switch / remove), sessions (create with hardware
  targeting + retry, list, stop), port forwarding, cost guards.
- Plugins: everything beyond the Colab core is a plugin. `llama-cpp` is bundled.
- A mock backend drives the whole UI without WSL, the CLI or a Google account.

Non-goals (v1)

- Installers, code signing, installing updates in place, plugin marketplace/registry.
- Native-code or WASM plugins. Plugins are declarative (manifest + scripts that run on the VM).
- A browser/WASM build. GPUI draws to a canvas with no DOM or accessibility tree there, the
  process-spawning backend cannot exist in WASM, and it would double the toolchain. UI tests use
  GPUI's headless `TestAppContext` instead (section 9).
- ADC / gcloud authentication. Only the CLI's default OAuth2 credential is managed.

## 2. Hard safety rules (apply to every contributor and agent)

1. **Never run a real `colab` command that touches the account or a runtime** during development
   or tests: nothing that authenticates, lists, creates, stops, executes or changes credentials
   (`sessions` prunes local state; most subcommands trigger an update check). The only real
   invocations allowed are `colab help [CMD]` and `colab version`, which do neither. The CLI is
   open source (github.com/googlecolab/google-colab-cli, installed version 0.8.0): read it
   instead of experimenting. Do not use the `colab-operator` skill to try things.
2. Debug builds default to the **mock** backend. The real backend needs `--real` (or
   `COLAB_PLAYGROUND_BACKEND=real`). Release builds default to real; `--mock` forces mock.
3. No test may spawn `wsl.exe`, except the `#[ignore]`d Windows smoke tests of the WSL bridge
   (`wsl_smoke`), which only run `printf`/`cat` round trips and never name `colab`.
4. The real backend is tested only against the **fake CLI** (`crates/core/tests/fake/`), on unix.
   The fake must never resolve to the system Python's `keyring` or a real `colab`.
5. All external side effects go through one seam: `Host` (process spawning) + `Toolchain`
   (paths of `colab` and its helper interpreter). Nothing else calls `std::process::Command`.
6. Never push to `main` directly. Every change lands through a pull request
   ([DEVELOPMENT.md](DEVELOPMENT.md#landing-a-change)).

## 3. Workspace layout

```
Cargo.toml                    workspace, members = ["crates/*"]
crates/core/                  colab-playground-core: no GPUI, all logic, fully testable
  assets/account_helper.py    keyring vault helper (runs with the CLI's own interpreter)
  assets/watchdog.py          VM-side deadline/idle watchdog
  tests/fake/                 fake `colab` + fake helper used by unix integration tests
crates/app/                   colab-playground: the GPUI + Ely binary
plugins/llama-cpp/            bundled plugin: plugin.toml + worker.py (embedded at build time)
docs/DESIGN.md  docs/PLUGINS.md  docs/COLAB_NOTES.md  docs/ROADMAP.md
docs/USAGE.md                 build/run, backend flags, demo data, where things are stored
docs/TESTING.md               automated tests, manual smoke checklist for real mode
README.md                     what it is, features, safety and cost; links into docs/
.github/workflows/ci.yml
```

Rust 1.98, edition 2024. `gpui = { package = "gpui-pre", version = "=0.3.12" }`,
`gpui_platform = { package = "gpui-pre-platform", version = "=0.3.12", features = ["font-kit", "wayland", "x11"] }`,
`ely-gpui-component` as a git dependency pinned by `rev`. Keep `.gitattributes` LF rules for
`*.py` and `*.sh`.

## 4. Core crate

No GPUI dependency. Blocking I/O is fine: the app runs core calls on GPUI's background
executor. Long operations report progress through a channel of typed events and accept a
cancellation flag. Time comes from an injected `Clock` so guard and retry logic are testable.

### 4.1 `host` — where commands run

```rust
pub enum Host { Local, Wsl { distro: Option<String> } }
```

- `Host::detect()`: `Wsl` on Windows, `Local` elsewhere. Everything Windows-specific lives here
  behind `cfg(windows)`: `wsl.exe`, `CREATE_NO_WINDOW`, `WSL_UTF8=1`.
- One entry point runs a POSIX script with argv, env, optional stdin, streaming stdout/stderr
  lines, and returns the exit status. Under WSL the script travels base64-encoded
  (`bash -lc 'eval "$(printf %s <b64> | base64 -d)"'`) so Windows→WSL quoting cannot corrupt it;
  values are passed as environment/positional args, never interpolated into script text.
- A login shell (`bash -lc`) is used so `~/.local/bin` is on `PATH`.
- Long-lived children (tunnels, login) are owned handles that kill the whole process tree on drop.
- Under WSL the real backend holds one idle `wsl.exe` (`HostKeepalive`, reading stdin until EOF)
  from its first CLI or helper call until it is dropped. WSL stops an idle distribution shortly
  after its last command, and the keyring daemon restarts locked with it, so background polls
  would otherwise keep asking for the keyring password. The app exiting closes the pipe, so the
  process never outlives it. A fresh distribution's D-Bus session bus (`unix:path=` in
  `DBUS_SESSION_BUS_ADDRESS`) comes up a moment after the first command, and before it the keyring
  is unreachable, so each start of the keepalive waits (bounded) for that socket before the CLI or
  helper call goes ahead.
- `Host::probe()` reports: WSL present / default distro (Windows), `bash`, `ssh`, `git`, `uv`,
  `python3`, `colab` path + version.

### 4.2 `cli` — typed wrapper over `colab`

Builds argv (global flags first: `--config <state file>`), runs it through `Host`, parses output.
Output formats come from the CLI source (github.com/googlecolab/google-colab-cli):

- `colab sessions`: `[name] endpoint | Hardware: X | Shape: Y | Variant: Z[ | Status: W]`,
  orphans show as `[?]`; "No active sessions found on server."
- `colab usage`: `Current balance: N compute units` / `Usage rate: N/hr` / `Active assignments: N`.
- `colab new -s NAME [--gpu G|--tpu T] [--high-mem]`; accelerator names are validated locally
  (0.8.0 rejects unknown ones; older versions silently fell back to A100). HTTP 400 with an
  accelerator = no entitlement/quota. Reusing an existing name orphans the previous VM, so names
  are always checked first.
- `colab exec -s NAME [--env K=V]... [--timeout S]` with the script on stdin; `colab stop -s NAME`.
- `colab ssh --proxy-mode -s NAME` is the OpenSSH `ProxyCommand`. A runtime allows **one**
  concurrent ssh (HTTP 429): the tunnel owns it; everything else uses `colab exec`.
- VM port 8080 is taken by Colab; reject it.

Errors are classified (`NotInstalled`, `AuthRequired`, `NotEntitled`, `SshBusy`, `SessionGone`,
`Other`) so the UI can localise them.

Install: `uv tool install google-colab-cli` when `uv` exists; otherwise offer to install `uv`
(official installer) first, or fall back to `pip install --user`. Streamed log, explicit user click.

### 4.3 `accounts` — login and switching

The CLI keeps exactly one OAuth token, in the OS keyring (`service=google-colab-cli`,
`username=oauth_token`), and has no logout. Login prints a URL on stderr and reads the
authorisation code from stdin (copy-paste flow; there is no localhost redirect).
Mechanism follows github.com/mizorewww/colab-account-changer (MIT):

- `assets/account_helper.py` runs with **the CLI's own interpreter** (resolved from the shebang
  of the real `colab` entry script), fed on stdin. It owns a vault in the same keyring
  (`service=colab-playground`, `username=account:<id>`). **Tokens never leave that Python
  process**; it prints only non-secret JSON (ids, emails, expiry, usage, errors).
- Operations: `status` (active email via tokeninfo), `list`, `adopt` (save the active token as an
  account), `use <id>` (copy vault → active slot), `remove <id>`, `usage <id>` (ccu-info, without
  switching), `begin-add` (back up the active token, clear the slot), `finish-add` (verify the
  new token, store it), `rollback-add` (restore the backup).
- Add account = `begin-add` → spawn `colab whoami` → capture the URL → the UI opens the browser
  and shows a code field → write the code to stdin → `finish-add`. Any failure or cancel runs
  `rollback-add`. The active token is never dropped without a restorable backup.
- Each account has its own session state file (`<host data>/accounts/<id>/sessions.json`, where
  `<host data>` is the data directory on the host where the CLI runs, inside WSL on Windows),
  passed with `--config` everywhere, including inside the ssh `ProxyCommand`. Otherwise
  `colab sessions` under account B prunes A's local entries. On adoption the CLI's default state file is copied,
  never modified.
- Non-secret metadata lives in `<data>/accounts.json`. A missing keyring backend is a visible,
  explained error state, not a crash.

Known limits (stated in the README's "Safety and cost"): switching changes the machine-wide CLI account; sessions on a
non-active account keep running (and billing) and are only protected by their VM-side watchdog.

### 4.4 `sessions` and `provision` — hardware targeting with retry

Colab picks the concrete machine: an A100 may be 40 GB or 80 GB and cannot be requested.

```rust
pub struct ProvisionSpec {
    pub name: String,
    pub accelerator: Accelerator,          // None | T4 | L4 | G4 | A100 | H100 | Tpu(..)
    pub high_mem: bool,
    pub min_vram_mib: Option<u32>,         // e.g. Some(70_000) to insist on the 80 GB A100
    pub max_attempts: u32,                 // default 3, hard cap 8
    pub retry_delay: Duration,             // default 20 s
    pub on_exhausted: OnExhausted,         // StopAndFail | KeepLast
    pub guard: GuardPolicy,
}
```

Loop: `new` → probe hardware with `colab exec` (`nvidia-smi` name + total MiB) → if it meets the
spec, start the watchdog and return; otherwise `stop`, wait, retry. Every attempt is an event
(`Allocating`, `Probed{gpu,vram}`, `Rejected`, `Retrying{n}`, `Ready`, `Failed`). A failed `new`
that still allocated a VM is stopped before retrying. `NotEntitled` aborts immediately (retrying
cannot help). Cancellation always leaves no VM behind.

### 4.5 `guard` — cost control

```rust
pub struct GuardPolicy {
    pub max_runtime: Option<Duration>,     // default 2 h
    pub max_spend_cu: Option<f64>,         // compute units for this session
    pub min_balance_cu: Option<f64>,       // stop everything below this account balance
    pub idle_timeout: Option<Duration>,    // VM-side: no heartbeat and GPU idle
}
```

Two independent layers, because an app-side timer dies with the app:

1. **VM-side watchdog** (`assets/watchdog.py`), started detached right after provisioning. It
   enforces an absolute deadline and the optional idle rule, then releases the runtime with
   `POST http://$TBE_RUNTIME_ADDR/unassign` (what `google.colab.runtime.unassign()` does). The app
   can extend or cancel the deadline by rewriting a small control file via `colab exec`; setting
   a deadline again after a cancel starts the watchdog again, so the stop is back on the runtime.
   The app shows whether a runtime's stop is on the runtime or only enforced while the app is open.
   Plugins signal activity by touching a heartbeat file.
2. **App-side guard**: evaluate a pure function
   `evaluate(policy, now, usage, session) -> Ok | Warn | Stop` on every usage sample.
   `Warn` at 80 % of any limit (toast + badge). `Stop` runs `colab stop` and records why.

### 4.5.1 Multiple sessions and live usage

- Several sessions may run at once on one account. Each has its own guard policy, watchdog,
  tunnels and plugin runs. `max_concurrent_sessions` (default 2, user-editable) caps how many the
  app will create, so a retry loop can never fan out into many billed VMs.
- `UsageSnapshot { balance_cu, rate_cu_per_hour, assignments, fetched_at }` per account, read
  through the account helper (ccu-info; works for any saved account without switching). Poll
  every 30 s while the active account has sessions, every 5 min otherwise, plus manual refresh.
  A failed fetch marks the snapshot **stale**; unknown is never treated as zero.
- Shown everywhere it matters: balance, burn rate (CU/h), time left (`balance / rate`), a short
  balance history, and totals across accounts on the Accounts page.
- Colab reports only account totals. A session's own rate is **estimated** from the rate change
  observed when it started or stopped (split evenly when ambiguous) and its spend is that rate
  integrated over time; the UI marks these as estimates (`≈`).
- If `assignments` exceeds the sessions the app knows, warn that unmanaged runtimes (e.g. opened
  in the browser) are consuming units; they are listed read-only.

Quitting with live sessions asks: stop them, or leave them under the watchdog. The UI always shows
balance, burn rate, per-session uptime, estimated spend and time to auto-stop.

### 4.5.2 Data that must survive the runtime — deferred

A stopped runtime is wiped. v1 says so wherever a stop is confirmed and ships only the hook
points for saving data: the watchdog's wind-down phase (grace period, `winddown` marker, hook
commands before unassign), `[[keep]]` and `needs` in the plugin manifest, secret slots in the
keyring vault, and `StopOptions` on the stop path, where a pre-stop step will be added. The full keep plan (local folder,
Hugging Face repo, Google Drive) is recorded in `docs/ROADMAP.md`.

### 4.6 `tunnel` — port forwarding

`ssh -N -L 127.0.0.1:<local>:127.0.0.1:<remote>` with
`-o ProxyCommand="colab --config <state> ssh --proxy-mode -s <name>"` given on the command line.
The user's `~/.ssh/config` is not edited. Generates `~/.ssh/id_ed25519` only if no key exists.
States: `Starting | Up | Reconnecting | Failed(reason) | Stopped`; exponential backoff; 429 and
"port in use" are distinct, localisable failures. Under WSL2 the forward is reachable from
Windows `localhost`. An optional HTTP health probe confirms the endpoint answers.

### 4.7 `plugins` — everything beyond the Colab core

A plugin is a directory with `plugin.toml` and scripts. Plugins **never execute anything on the
user's machine**: they declare a script that runs on the Colab VM and the ports to expose.

```toml
api = 1
id = "llama-cpp"
version = "0.1.0"
[name]         en = "llama.cpp server"   zh-CN = "…"   zh-TW = "…"
[description]  en = "…"
[provision]    accelerator = "A100"  min_vram_mib = 0  high_mem = false   # defaults, user-editable
[[inputs]]     id = "repo"  type = "hf-model"  required = true   [inputs.label] en = "Model repo"
[[inputs]]     id = "quant" type = "select" options = ["auto","Q8_0","Q6_K","Q5_K_M","Q4_K_M"] default = "auto"
[[inputs]]     id = "port"  type = "number" default = 8000
[run]          script = "worker.py"
[[stages]]     id = "download"  [stages.label] en = "Download model"
[[endpoints]]  id = "api"  port_input = "port"  path = "/v1"  health = "/health"  kind = "openai"
```

- Input types: `text`, `number`, `bool`, `select`, `secret`, `hf-model` (host-side Hugging Face
  search + file listing). Inputs reach the script as `PLUGIN_INPUT_<ID>` via `colab exec --env`;
  secrets are never written to disk or logs.
- Status protocol: the script writes `status.json` atomically in its run directory:
  `{ "state": "running|ready|failed", "stage": "<id>", "progress": 0.0-1.0?, "message": "…",
     "outputs": { "model_id": "…", … } }`. The host polls it with `colab exec`, renders stages,
  log tail and outputs generically, and turns declared endpoints into tunnels.
- Host services to a run: target session (existing or freshly provisioned with section 4.4),
  heartbeat path for the watchdog, run directory, log files.
- Bundled plugins are embedded in the binary; user plugins install into `<data>/plugins/<id>/`
  from a local folder or a Git URL (host `git clone --depth 1`), with a confirmation that shows
  the manifest and states that its script will run on the user's paid runtime. Enable, disable,
  remove. Manifest validation is strict and versioned by `api`.

### 4.8 The bundled `llama-cpp` plugin

Port of the existing `llm-deploy/worker.py` (keep every lesson from `docs/COLAB_NOTES.md`:
prebuilt CUDA binaries, `LD_LIBRARY_PATH=<bindir>:/usr/lib64-nvidia`, never port 8080, detached
`Popen(start_new_session=True)`, warm-up request, VRAM-used check against silent CPU fallback).
Changes: English messages, the status protocol above, and **quantisation fitting**:

- `quant = auto`: list the repo's GGUF files, read the detected VRAM, and pick the highest-quality
  quant whose estimated footprint (file size + context/KV estimate + margin) fits; report the
  choice and the reason in `outputs`. A fixed quant that cannot fit fails fast with the list of
  quants that would.
- Existing fallbacks stay: KV `f16 → q8_0`, MTP on → off on start failure.

So "A100, prefer 80 GB, up to 3 attempts, otherwise keep 40 GB" + `quant = auto` yields Q8_0 on
80 GB and a smaller quant on 40 GB without user intervention.

### 4.9 `backend` — what the UI talks to

`trait Backend` covers probe/install, accounts, sessions/provision, guard, tunnels, plugins and
Hugging Face lookup. `RealBackend` composes the modules above. `MockBackend` is a deterministic,
clock-driven simulation (scripted install log, login URL + code, A100 40 GB then 80 GB on retry,
usage that burns down, plugin run through all stages, tunnel up) with switches to inject each
failure. It is the default in debug builds and the only backend used by UI tests.

## 5. App crate

- Window chrome: no OS title bar. The app draws its own title bar from Ely's shell components so
  it follows the theme and blends with the sidebar: Windows gets themed minimise / maximise /
  close buttons with native behaviour (drag, double-click to maximise, edge resize, Aero Snap and
  the Windows 11 snap-layout flyout); macOS keeps the native traffic lights over a transparent
  title bar and shows no app mark or name beside them (the menu bar names the app); Linux uses
  client-side decorations where the compositor expects them. Platform differences are
  `cfg`-isolated.
- Shell: sidebar navigation (Overview, Sessions, Plugins, Accounts, Settings), account switcher,
  always-visible balance / burn-rate badge, toasts.
- Overview: first-run checklist (environment → CLI installed → signed in) and quick actions;
  bundled plugins appear as "Quick deploy" cards.
- Sessions: table with hardware, uptime, estimated spend, auto-stop countdown; new-session dialog
  (accelerator, high-mem, minimum VRAM, attempts, guard policy); forward a port; extend; stop.
- Plugins: installed list, install from folder / Git URL, run form generated from the manifest,
  run view (stages, log, outputs) and endpoint cards (URL, copy, health, tunnel toggle).
- Accounts: list with usage, add (opens browser, paste code), switch, remove.
- Settings: language (System / English / 简体中文 / 繁體中文), appearance (System / Light / Dark),
  WSL distro, `colab` path, and **Defaults**. Persisted as JSON in the OS config dir.
- **No policy value is hard-coded.** Every default lives in one serde `Defaults` struct in core
  (max runtime 2 h, spend cap, balance floor, idle timeout, `max_concurrent_sessions` 2,
  provisioning attempts 3, retry delay, `on_exhausted` = stop, usage poll intervals, default
  accelerator). Settings edits them; the new-session dialog and every plugin run form are
  pre-filled from them and can override each value for that one action.
- i18n: Ely's `I18n` global plus the app's own catalog. A unit test asserts identical key sets
  for `en-US`, `zh-CN`, `zh-TW`. No user-visible literal outside the catalog.
- Theme: Ely `Theme::set_mode`; System follows the window appearance and its changes.
- All colours, radii and sizes come from `cx.theme()`.

## 6. Verification

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Core unit tests: parsers, argv builders, base64 script transport, guard decisions, provisioning
  loop, manifest validation, quant fitting.
- Core integration tests (`cfg(unix)`): `RealBackend` against the fake CLI — install detection,
  login + rollback, switch, per-account `--config`, provision retry, plugin run, tunnel argv.
- App UI tests: `gpui::TestAppContext` + `MockBackend` — navigation, locale and theme switching,
  login flow, provision retry to an 80 GB A100, quick deploy to a ready endpoint, guard auto-stop,
  plugin install from a fixture folder.
- CI (light): Linux runs fmt, clippy and all tests; Windows and macOS run `cargo check
  --workspace --all-targets`, the core tests and the app's UI tests.
- Not verifiable by automated tests (covered by the manual smoke checklist in `docs/TESTING.md`):
  real-account login/switch, real provisioning, the VM-side `unassign`, real tunnels.
