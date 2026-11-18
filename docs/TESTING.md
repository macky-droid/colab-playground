# Testing Colab Playground

What the automated tests cover, and the manual checklist for the part they cannot.

## Automated tests

The commands CI runs on each platform are in
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml); run the same ones locally.

- UI tests (`crates/app/tests/ui/`) render the real views headless under GPUI's `TestAppContext`,
  with the mock backend only.
- `RealBackend` is tested against a fake `colab`, keyring and `ssh` (`crates/core/tests/fake/`).
  Those tests are unix-only: they run on Linux and macOS, not on Windows.
- No test runs the real `colab` ([hard safety rules](DESIGN.md#hard-safety-rules)). The ignored,
  Windows-only `wsl_smoke` tests only round-trip `printf`/`cat` through WSL and hold an idle
  keepalive: `cargo test -p colab-playground-core --test wsl_smoke -- --ignored`.

## Manual smoke checklist (real mode)

Because no automated test may touch a Google account or a runtime, real mode is checked by hand
with this list: sign-in and switching, provisioning, tunnels and the auto-stop on the runtime. Run
it after changing any of those, and before trusting a build with paid compute.

Go in order, cheapest first. At the first surprise, stop anything that is billing, keep the
terminal log and note what you saw.

Before you start: close runtimes opened in the browser, open the Colab web UI's resources page
(compute units and active sessions) and note the balance. Run
`cargo run -p colab-playground -- --real` from a terminal; the **Demo data** badge must be absent
and Settings → About → Backend must read "Colab CLI".

1. **CLI detection or install** (free). Overview → "Get set up" should show "Using `<distro>`.",
   "bash and ssh are available." and "Version … is installed.". If the CLI is missing, choose
   "uv (recommended)" and click Install; the log should end with "The Colab CLI is installed."
2. **Sign in** (free). If the CLI is already signed in, the checklist offers "Use this account";
   expect "Saved `<email>`". Otherwise, or for a second account: Accounts → "Add account" → "Open
   the sign-in page", sign in in the browser, paste the code into "Authorization code", "Sign in";
   expect "Signed in as `<email>`" and the new account active. The add flow empties the CLI's
   sign-in slot while it runs (with a backup): with an account already active, cancel once and
   enter a wrong code once; each time expect "Your previous account is still the active one."
   "Can't reach the system keyring" means the keyring inside WSL is locked or missing.
3. **Usage** (free). The balance badge, Overview and Accounts should match the Colab web UI, with
   the burn rate "Not in use"; the refresh button should update them.
4. **CPU runtime** (first billed step). Sessions → "New session", hardware "CPU only", "Stop and
   end billing after" 15 minutes, "Start session". Expect "Asking Colab for a runtime…", then
   "Auto-stop is set on the runtime; it works even if this app is closed." and "`<name>` is ready.".
   The card shows the auto-stop countdown and the Colab web UI lists the runtime. Try "Extend
   by …". Then "Cancel auto-stop", confirm, and "Set auto-stop in …" on the same card: expect
   "Auto-stop is set on the runtime…" again (the watchdog is restarted) and no "applies only while
   this app is open" note on the card. If "Couldn't set the automatic stop on the runtime" appears
   at any point, stop the runtime now and skip step 10.
5. **Forward a port.** On that card, "Forward a port" (runtime port 8000, any free local port, no
   health path). Expect "Connecting", then "Up": the `colab ssh` tunnel works. Nothing listens on
   the runtime yet, so the address does not answer (with a health path: "Up, not answering").
   "Up" is detected on this computer, so on Windows a forward that fails with "This took too
   long." points at WSL's localhost forwarding rather than ssh. "Close forward" should report that
   the runtime keeps billing.
6. **Stop it.** "Stop and end billing", confirm. Expect "`<name>` is stopped. Its billing has ended."
   and an "Ended recently" entry. After "Refresh" it must be gone from Sessions and from the Colab
   web UI, and the burn rate back to "Not in use".
7. **Switch account and back** (two saved accounts). With nothing running the dialog says "Nothing
   is running on the current account, so nothing else changes."; expect "Now using `<email>`" and
   that account's balance, then switch back. Optional: repeat with a 15-minute CPU runtime and a
   forward on the first account; the dialog should list both, and after the switch stop that
   runtime from Sessions without switching back.
8. **GPU retry.** Cheapest first: "NVIDIA T4", minimum GPU memory 20 GB (more than a T4 has), 2
   attempts, "Stop and end billing" if no attempt matches. Expect "Got …", "… is less than the
   20 GB minimum, so it is given back.", "Waiting 20 s before trying again…", the same again, then
   "No attempt got the requested hardware (2 tries)." with nothing left running. Only then, if
   wanted, the real lottery: "NVIDIA A100", minimum 70 GB, 3 attempts; stop it as soon as it is
   ready.
9. **llama.cpp plugin, small model.** Overview → "Deploy a model". Model repository
   `unsloth/Qwen3-0.6B-GGUF`, quantisation `auto`; change the hardware from A100 to T4, auto-stop
   30 minutes, keep "Stop the new session if this fails" on, "Deploy". Expect the stages "Check the
   GPU", "Choose a quantisation", "Download the model", "Start the server", "Warm up", then "Ready.
   Its addresses are below." Turn on "Forward to this computer" and expect "Answering"; open the
   web chat or use "Try it". Then "Stop and end billing". Do not restart the app mid-run: the run
   view is not recovered after a restart (see "Plugin runs" in [ROADMAP.md](ROADMAP.md)).
10. **Auto-stop with the app closed.** Start a CPU runtime with a 10-minute auto-stop and check
    that "Auto-stop is set on the runtime…" appears again. Quit and choose "Leave running and
    quit". After the deadline plus 2 minutes the runtime must be gone from the Colab web UI and,
    after restarting the app, from Sessions. If it is still there 5 minutes past the deadline,
    stop it by hand: the watchdog did not release it.

**Afterwards, make sure nothing is billing:** Sessions shows "Nothing is running" with no "Couldn't
list runtimes on …" error for any account, the burn rate reads "Not in use", and the Colab web UI's
resources page shows no active session for every account you used.
