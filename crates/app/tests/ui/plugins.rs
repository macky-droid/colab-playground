//! Plugins over the headless mock: quick deploy to a ready endpoint, a run failing at a stage,
//! installing from a fixture folder (with the confirmation), and secrets staying secret.

use colab_playground::{
    i18n::keys,
    route::Route,
    state::{InstallPrompt, Load, Overlay, PluginView, RunPhase},
    ui::{
        overlays::INSTALL_CONFIRM,
        pages::{
            overview::{INSTALL_SELECTOR, deploy_selector},
            plugins::{
                ENDPOINT_SELECTOR, INSTALL_CHECK, RUN_START, RUN_STOP, hf_result_selector,
                plugin_selector,
            },
        },
    },
};
use colab_playground_core::{
    CancelToken, ErrorKind,
    backend::Backend,
    mock::{MockFailures, MockScenario},
    tunnel::TunnelState,
};
use gpui::TestAppContext;

use crate::support::{Options, TestApp};

const REPO: &str = "unsloth/Qwen3-0.6B-GGUF";

fn run_phase(app: &mut TestApp<'_>) -> Option<RunPhase> {
    app.read(|state, _| state.plugin_run().map(|run| run.phase.clone()))
}

/// Quick deploy from the Overview: the run form opens pre-filled.
fn open_deploy_form(app: &mut TestApp<'_>) {
    assert!(
        app.shown(&deploy_selector("llama-cpp")),
        "a Deploy a model card"
    );
    app.click(&deploy_selector("llama-cpp"));
    assert_eq!(app.route(), Route::Plugins);
    assert_eq!(app.read(|state, _| state.plugin_view()), PluginView::Form);
    let quant = app.read(|state, _| {
        state
            .run_form()
            .and_then(|form| form.values.get("quant").cloned())
    });
    assert_eq!(
        quant,
        Some(colab_playground_core::plugins::InputValue::Text(
            "auto".into()
        )),
        "manifest defaults are filled in"
    );
}

#[gpui::test]
fn quick_deploy_runs_to_a_ready_endpoint(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    open_deploy_form(&mut app);

    // Search, pick a result, see which quantisation fits.
    app.fill(|inputs| inputs.hf_query.clone(), "qwen3");
    app.click("hf-search");
    assert!(app.shown(&hf_result_selector(REPO)), "search results");
    app.click(&hf_result_selector(REPO));
    let repo = app.read(|state, _| state.run_form().and_then(|f| f.hf.repo.clone()));
    assert_eq!(repo.as_deref(), Some(REPO));
    assert!(app.shown("quant-preview 40960"), "an A100 can be 40 GB…");
    assert!(app.shown("quant-preview 81920"), "…or 80 GB");

    app.click(RUN_START);
    let Some(RunPhase::Ready(outcome)) = run_phase(&mut app) else {
        panic!("expected a ready run, got {:?}", run_phase(&mut app));
    };
    assert_eq!(app.read(|state, _| state.plugin_view()), PluginView::Run);
    assert!(!outcome.endpoints.is_empty());
    let endpoint = &outcome.endpoints[0];
    assert!(app.shown(&format!("{ENDPOINT_SELECTOR} {}", endpoint.id)));
    let forwards: Vec<(u16, TunnelState)> = app.read(|state, _| {
        state
            .tunnels()
            .map(|t| (t.spec.remote_port, t.state.clone()))
            .collect()
    });
    assert_eq!(
        forwards,
        [(endpoint.remote_port, TunnelState::Up)],
        "the endpoint is forwarded"
    );
    let stages = app.read(|state, _| state.plugin_run().and_then(|r| r.status.clone()));
    assert_eq!(
        stages.map(|s| s.state),
        Some(colab_playground_core::vm::RunState::Ready)
    );
}

#[gpui::test]
fn a_failed_stage_says_whether_the_runtime_still_bills(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            plugin_fail_stage: Some("download".into()),
            ..MockFailures::default()
        },
        ..Options::scenario(MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    open_deploy_form(&mut app);
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        REPO,
    );
    app.click(RUN_START);
    let Some(RunPhase::Failed(_)) = run_phase(&mut app) else {
        panic!("expected a failure, got {:?}", run_phase(&mut app));
    };
    assert!(app.shown("error-run"));
    assert_eq!(
        app.read(|state, _| state.plugin_run().and_then(|r| r.session_stopped())),
        Some(true),
        "stop on failure is on by default"
    );
    let left = app.mock.runtimes(&CancelToken::new()).expect("listing");
    assert!(left.runtimes.is_empty(), "nothing bills: {left:?}");

    // Again, keeping the session: it is still billing and Stop is right there.
    app.click("run-again");
    app.update(|state, cx| state.edit_run_form(|form| form.stop_on_failure = false, cx));
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        REPO,
    );
    app.click(RUN_START);
    assert!(matches!(run_phase(&mut app), Some(RunPhase::Failed(_))));
    let left = app.mock.runtimes(&CancelToken::new()).expect("listing");
    assert_eq!(left.runtimes.len(), 1, "the session was kept");
    assert!(app.shown(RUN_STOP), "with Stop right there");
    app.click(RUN_STOP);
    app.click(colab_playground::ui::overlays::STOP_CONFIRM);
    let left = app.mock.runtimes(&CancelToken::new()).expect("listing");
    assert!(left.runtimes.is_empty());
}

#[gpui::test]
fn a_runtime_is_only_called_stopped_when_its_account_was_listed(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            plugin_fail_stage: Some("download".into()),
            ..MockFailures::default()
        },
        ..Options::scenario(MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    open_deploy_form(&mut app);
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        REPO,
    );
    app.click(RUN_START);
    assert!(matches!(run_phase(&mut app), Some(RunPhase::Failed(_))));
    assert!(
        app.shown("run-session-stopped"),
        "the listing no longer has it"
    );
    // The run's account cannot be listed now: whether its runtime is gone is not known.
    app.set_failures(MockFailures {
        unlisted_accounts: vec!["ada@example.com".into()],
        ..MockFailures::default()
    });
    app.update(|state, cx| state.load_runtimes(cx));
    assert!(!app.shown("run-session-stopped"));
    assert!(app.shown(colab_playground::ui::pages::plugins::RUN_SESSION_UNKNOWN_SELECTOR));
}

const FIXTURE_MANIFEST: &str = r#"api = 1
id = "hello-web"
version = "0.2.0"
needs = ["hf_token"]

[name]
en = "Hello web server"
zh-CN = "你好网页服务"
zh-TW = "你好網頁服務"

[description]
en = "Serves a page that says hello."

[[inputs]]
id = "greeting"
type = "text"
default = "hello"
[inputs.label]
en = "Greeting"

[run]
script = "run.sh"

[[stages]]
id = "start"
[stages.label]
en = "Start"

[[endpoints]]
id = "web"
port = 9000
[endpoints.label]
en = "Web page"
"#;

#[gpui::test]
fn a_plugin_from_a_folder_is_installed_after_confirming(cx: &mut TestAppContext) {
    let fixture = tempfile::tempdir().expect("a fixture folder");
    std::fs::write(fixture.path().join("plugin.toml"), FIXTURE_MANIFEST).expect("manifest");
    std::fs::write(fixture.path().join("run.sh"), "#!/bin/bash\necho hello\n").expect("script");
    let mut app = TestApp::new(cx);
    app.click(Route::Plugins.nav_selector());
    assert!(
        app.shown(&plugin_selector("llama-cpp")),
        "the built-in plugin is listed"
    );

    let folder = fixture.path().display().to_string();
    app.fill(|inputs| inputs.plugin_source.clone(), &folder);
    app.click(INSTALL_CHECK);
    let Some(Overlay::InstallPlugin(InstallPrompt::Preview(preview))) =
        app.read(|state, _| state.overlay().cloned())
    else {
        panic!(
            "expected the confirmation, got {:?}",
            app.read(|state, _| state.overlay().cloned())
        );
    };
    assert_eq!(preview.manifest.id, "hello-web");
    assert!(
        app.read(|state, _| state.plugin("hello-web").is_none()),
        "nothing is installed before confirming"
    );
    assert!(app.shown(INSTALL_CONFIRM));

    app.click(INSTALL_CONFIRM);
    assert!(app.read(|state, _| state.overlay().is_none()));
    assert!(app.read(|state, _| state.plugin("hello-web").is_some()));
    assert!(app.shown(&plugin_selector("hello-web")));
    let installed = app.cx.update(|_, cx| {
        colab_playground::i18n::tr_args(cx, keys::PLUGIN_INSTALLED, &[("name", "Hello web server")])
    });
    assert!(app.toasts().contains(&installed), "{:?}", app.toasts());

    // It can be removed (after confirming); built-in plugins cannot.
    app.click("plugin-remove hello-web");
    app.click(colab_playground::ui::overlays::REMOVE_CONFIRM);
    assert!(app.read(|state, _| state.plugin("hello-web").is_none()));
}

#[gpui::test]
fn secrets_never_show_up_anywhere(cx: &mut TestAppContext) {
    const SECRET: &str = "sk-very-secret-4711";
    const TOKEN: &str = "hf_token-secret-0815";
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));

    // The Hugging Face token: saved from the masked field, which is then emptied.
    app.click(Route::Plugins.nav_selector());
    app.fill(|inputs| inputs.hf_token.clone(), TOKEN);
    app.click("hf-token-save");
    assert_eq!(
        app.read(|state, _| state.hf_token().ready().copied()),
        Some(true)
    );
    let field = app.read_root(|root, cx| root.inputs().hf_token.read(cx).text().to_owned());
    assert!(field.is_empty(), "the field is cleared");

    // A secret run input.
    open_deploy_form_from_plugins(&mut app);
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        REPO,
    );
    app.fill(
        |inputs| inputs.field("api_key").expect("the key field"),
        SECRET,
    );
    app.click(RUN_START);
    assert!(matches!(run_phase(&mut app), Some(RunPhase::Ready(_))));

    let field = app.read_root(|root, cx| {
        root.inputs()
            .field("api_key")
            .map(|f| f.read(cx).text().to_owned())
            .unwrap_or_default()
    });
    assert!(
        field.is_empty(),
        "the secret field is cleared after starting"
    );
    let seen = app.read(|state, _| {
        format!(
            "{:?}\n{:?}\n{:?}",
            state.plugin_run(),
            state.run_form(),
            state.tunnels().collect::<Vec<_>>()
        )
    });
    for secret in [SECRET, TOKEN] {
        assert!(
            !seen.contains(secret),
            "{secret} leaked into state or events"
        );
        assert!(
            !app.toasts().iter().any(|t| t.contains(secret)),
            "{secret} leaked into a toast"
        );
        if let Some(file) = app.settings_file() {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            assert!(!text.contains(secret), "{secret} leaked into settings.json");
        }
    }

    // Running again asks for the secret again.
    app.click("run-again");
    let field = app.read_root(|root, cx| {
        root.inputs()
            .field("api_key")
            .map(|f| f.read(cx).text().to_owned())
            .unwrap_or_default()
    });
    assert!(field.is_empty());
}

fn open_deploy_form_from_plugins(app: &mut TestApp<'_>) {
    app.click("plugin-run llama-cpp");
    assert_eq!(app.read(|state, _| state.plugin_view()), PluginView::Form);
}

#[gpui::test]
fn cancelling_a_deployment_leaves_nothing_billing(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    open_deploy_form(&mut app);
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        REPO,
    );
    // Wait 2 of the run is the hardware probe of its fresh session: a VM exists by then.
    app.cancel_switch.arm(2);
    app.click(RUN_START);
    assert_eq!(run_phase(&mut app), Some(RunPhase::Cancelled));
    let provision = app.read(|state, _| {
        state
            .plugin_run()
            .map(|run| run.provision.events.clone())
            .unwrap_or_default()
    });
    assert!(
        provision.iter().any(|e| matches!(
            e,
            colab_playground_core::provision::ProvisionEvent::Releasing { .. }
        )),
        "the session it allocated was released: {provision:?}"
    );
    let left = app.mock.runtimes(&CancelToken::new()).expect("listing");
    assert!(left.runtimes.is_empty(), "nothing bills: {left:?}");
}

#[gpui::test]
fn plugins_are_listed_before_the_cli_is_installed(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            cli_missing: true,
            ..MockFailures::default()
        },
        ..Options::scenario(MockScenario::signed_out())
    };
    let mut app = TestApp::with(cx, options);
    app.click(Route::Plugins.nav_selector());
    assert!(
        app.shown(&plugin_selector("llama-cpp")),
        "the list does not wait for the CLI"
    );
    let token = app.read(|state, _| state.hf_token().clone());
    assert!(
        matches!(&token, Load::Failed(error) if error.is(ErrorKind::NotInstalled)),
        "the keyring cannot be read yet: {token:?}"
    );

    app.click(Route::Overview.nav_selector());
    app.click(INSTALL_SELECTOR);
    assert_eq!(
        app.read(|state, _| state.hf_token().clone()),
        Load::Ready(false),
        "checked once the CLI is there"
    );
}

#[gpui::test]
fn accounts_cannot_change_while_a_deployment_runs(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    let lin = app.read(|state, _| {
        state.accounts().ready().and_then(|view| {
            view.accounts
                .iter()
                .find(|a| a.email.as_deref() == Some("lin@example.com"))
                .map(|a| a.id.clone())
        })
    });
    let lin = lin.expect("the demo's second account");
    app.click(Route::Plugins.nav_selector());
    app.click("plugin-run llama-cpp");
    app.update(|state, cx| {
        // Pick an existing session so the run does not hit the session limit.
        state.edit_run_form(|form| form.existing = Some("a100-7f3a".into()), cx);
        state.start_run(vec![("repo".into(), REPO.into())], String::new(), cx);
        assert!(state.is_running_plugin());
        assert!(state.account_change_block().is_some());
        state.request_switch(lin.clone(), cx);
        state.open_add_account(cx);
        assert!(
            state.dialog().is_none(),
            "neither a switch nor a sign-in starts"
        );
    });
    let blocked = app.text(keys::ACCOUNT_CHANGE_BLOCKED);
    assert!(app.toasts().contains(&blocked), "{:?}", app.toasts());
}

// Several iterations shuffle task scheduling, so the older search can finish last.
#[gpui::test(iterations = 12)]
fn only_the_latest_search_is_shown(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    open_deploy_form(&mut app);
    app.update(|state, cx| {
        state.search_hf("qwen3".into(), cx);
        state.search_hf("8b".into(), cx);
    });
    app.settle();
    let results: Vec<String> = app.read(|state, _| {
        state
            .run_form()
            .and_then(|form| form.hf.results.ready().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|model| model.id)
            .collect()
    });
    assert!(!results.is_empty());
    assert!(
        results
            .iter()
            .all(|id| id.to_ascii_lowercase().contains("8b")),
        "results match the last query only: {results:?}"
    );
}
