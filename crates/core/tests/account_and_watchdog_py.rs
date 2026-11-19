#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use colab_playground_core::{CancelToken, Host};

#[test]
fn account_helper_ccu_parsing_keyring_guard_and_slot_cas() {
    let tmp = tempfile::tempdir().unwrap();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/account_helper.py");
    let driver = tmp.path().join("driver.py");
    std::fs::write(
        &driver,
        r#"import json, os, sys, tempfile, types

helper_path = sys.argv[1]
with open(helper_path, "rb") as fh:
    raw = fh.read()
assert b"\r" not in raw, "account_helper.py must keep LF line endings"

ns = {"__name__": "account_helper_test"}
exec(compile(raw.decode("utf-8"), helper_path, "exec"), ns)
Fail = ns["Fail"]
parse_ccu_info = ns["parse_ccu_info"]

assert parse_ccu_info({
    "currentBalance": None,
    "paidComputeUnitsBalance": 42.5,
    "consumptionRateHourly": 11.77,
    "assignmentsCount": 1,
}) == {"balance": 42.5, "rate": 11.77, "assignments": 1}

assert parse_ccu_info({
    "paid_balance": 10.0,
    "free_tokens_balance": 25.5,
    "consumptionRateHourly": None,
    "assignments": [{"endpoint": "e1"}, {"endpoint": "e2"}],
}) == {"balance": 35.5, "rate": 0.0, "assignments": 2}

assert parse_ccu_info({
    "currentBalance": None,
    "paidComputeUnitsBalance": None,
    "freeCcuQuotaInfo": {"remainingTokens": 100},
    "consumptionRateHourly": None,
    "assignmentsCount": None,
}) == {"balance": 0.0, "rate": 0.0, "assignments": 0}

for bad in (
    {"currentBalance": True, "consumptionRateHourly": 0, "assignmentsCount": 0},
    {"currentBalance": float("nan"), "consumptionRateHourly": 0, "assignmentsCount": 0},
    {"unrelated": 1},
):
    try:
        parse_ccu_info(bad)
        raise AssertionError("expected Fail('parse') for %r" % (bad,))
    except Fail as exc:
        assert exc.kind == "parse", exc.kind

class PlaintextKeyring:
    pass
PlaintextKeyring.__module__ = "keyrings.alt.file"

class SafeKeyring:
    pass

class ChainerBackend:
    def __init__(self, backends):
        self.backends = backends
ChainerBackend.__module__ = "keyring.backends.chainer"

store = {}
state = {"kr": PlaintextKeyring()}
kr_mod = types.ModuleType("keyring")
kr_mod.get_keyring = lambda: state["kr"]
kr_mod.get_password = lambda s, u: store.get((s, u))
kr_mod.set_password = lambda s, u, v: store.__setitem__((s, u), v)
kr_mod.delete_password = lambda s, u: store.pop((s, u), None)
sys.modules["keyring"] = kr_mod

os.environ.pop("COLAB_PLAYGROUND_ALLOW_PLAINTEXT_KEYRING", None)
for backend in (
    PlaintextKeyring(),
    ChainerBackend([PlaintextKeyring()]),
    ChainerBackend([SafeKeyring(), PlaintextKeyring()]),
    ChainerBackend([]),
):
    state["kr"] = backend
    try:
        ns["_keyring"]()
        raise AssertionError("expected keyring_unavailable for %r" % (backend,))
    except Fail as exc:
        assert exc.kind == "keyring_unavailable"

os.environ["COLAB_PLAYGROUND_ALLOW_PLAINTEXT_KEYRING"] = "1"
state["kr"] = ChainerBackend([SafeKeyring(), PlaintextKeyring()])
assert ns["_keyring"]() is kr_mod
os.environ.pop("COLAB_PLAYGROUND_ALLOW_PLAINTEXT_KEYRING", None)

state["kr"] = SafeKeyring()

def tok(name, access):
    return json.dumps({
        "token": access,
        "refresh_token": "rt-" + name,
        "client_id": "cid",
        "client_secret": "csec",
        "token_uri": "https://oauth2.googleapis.com/token",
    })

with tempfile.TemporaryDirectory() as lock_dir:
    os.environ["CPG_FAKE_STATE"] = lock_dir
    alice_old = tok("alice", "old-alice")
    alice_new = tok("alice", "new-alice")
    alice_reauth = tok("alice-reauth", "reauth-alice")
    bob_cur = tok("bob", "cur-bob")

    store.clear()
    store[("google-colab-cli", "oauth_token")] = alice_old
    store[("colab-playground", "account:alice")] = alice_old
    store[("colab-playground", "account:bob")] = bob_cur

    def refresh_during_switch(raw):
        store[("google-colab-cli", "oauth_token")] = bob_cur
        return types.SimpleNamespace(token="new-alice"), alice_new

    ns["refreshed"] = refresh_during_switch
    creds = ns["credential_for"]("alice", alice_old)
    assert creds.token == "new-alice"
    assert store[("google-colab-cli", "oauth_token")] == bob_cur
    assert store[("colab-playground", "account:alice")] == alice_new

    store.clear()
    store[("google-colab-cli", "oauth_token")] = bob_cur
    store[("colab-playground", "account:alice")] = alice_old

    def refresh_during_remove(raw):
        store.pop(("colab-playground", "account:alice"), None)
        return types.SimpleNamespace(token="new-alice"), alice_new

    ns["refreshed"] = refresh_during_remove
    creds = ns["credential_for"]("alice", bob_cur)
    assert creds.token == "new-alice"
    assert ("colab-playground", "account:alice") not in store

    store.clear()
    store[("google-colab-cli", "oauth_token")] = bob_cur
    store[("colab-playground", "account:alice")] = alice_old

    def refresh_during_reauth(raw):
        store[("colab-playground", "account:alice")] = alice_reauth
        return types.SimpleNamespace(token="new-alice"), alice_new

    ns["refreshed"] = refresh_during_reauth
    creds = ns["credential_for"]("alice", bob_cur)
    assert creds.token == "new-alice"
    assert store[("colab-playground", "account:alice")] == alice_reauth

    bad_lock_root = os.path.join(lock_dir, "not_a_dir")
    with open(bad_lock_root, "w") as fh:
        fh.write("x")
    os.environ["CPG_FAKE_STATE"] = bad_lock_root
    with ns["slot_file_lock"](blocking=False) as acquired:
        assert acquired is False
    try:
        with ns["slot_file_lock"](blocking=True):
            raise AssertionError("expected Fail('io') when lock file cannot be opened")
    except Fail as exc:
        assert exc.kind == "io", exc.kind
        assert exc.__cause__ is None

print("OK")
"#,
    )
    .unwrap();

    let out = Host::Local
        .run_program(
            "python3",
            &[
                "-B".to_owned(),
                driver.to_string_lossy().into_owned(),
                helper.to_string_lossy().into_owned(),
            ],
            &[("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned())],
            &CancelToken::new(),
            Some(Duration::from_secs(30)),
        )
        .unwrap();
    assert!(out.success(), "{}\n{}", out.stdout, out.stderr);
    assert_eq!(out.stdout.trim(), "OK");
}

#[test]
fn watchdog_treats_cpu_tpu_loadavg_as_busy_without_nvidia_smi() {
    let tmp = tempfile::tempdir().unwrap();
    let watchdog = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/watchdog.py");
    let driver = tmp.path().join("driver.py");
    std::fs::write(
        &driver,
        r#"import json, os, sys, tempfile

wd_path = sys.argv[1]
with open(wd_path, "rb") as fh:
    raw = fh.read()
assert b"\r" not in raw, "watchdog.py must keep LF line endings"

ns = {"__name__": "watchdog_test"}
exec(compile(raw.decode("utf-8"), wd_path, "exec"), ns)
Watchdog = ns["Watchdog"]

with tempfile.TemporaryDirectory() as d:
    cfg = os.path.join(d, "watchdog.json")
    loadavg = os.path.join(d, "loadavg")
    with open(cfg, "w") as fh:
        json.dump({"started": 100.0}, fh)
    with open(loadavg, "w") as fh:
        fh.write("0.85 0.40 0.20 2/120 999\n")

    wd = Watchdog(cfg, loadavg_path=loadavg)
    wd.load()
    wd.gpu_utilisation = lambda: None

    assert wd.idle_for(500.0) == 0.0

    with open(loadavg, "w") as fh:
        fh.write("0.02 0.05 0.05 1/120 1000\n")
    assert wd.idle_for(560.0) == 60.0

print("OK")
"#,
    )
    .unwrap();

    let out = Host::Local
        .run_program(
            "python3",
            &[
                "-B".to_owned(),
                driver.to_string_lossy().into_owned(),
                watchdog.to_string_lossy().into_owned(),
            ],
            &[("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned())],
            &CancelToken::new(),
            Some(Duration::from_secs(30)),
        )
        .unwrap();
    assert!(out.success(), "{}\n{}", out.stdout, out.stderr);
    assert_eq!(out.stdout.trim(), "OK");
}
