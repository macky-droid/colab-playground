//! Plugin runs: the run form generated from the manifest (pre-filled by
//! `plugin_run_template`), Hugging Face search and file listing, and the run view (stages,
//! progress, log, outputs, endpoints).
//!
//! Secret inputs never enter this state: they are read from their masked fields when the run
//! starts, sent with that one request, and the fields are cleared (a second run asks again).

use std::{collections::VecDeque, sync::Arc};

use colab_playground_core::{
    Error, ErrorKind,
    accounts::AccountId,
    backend::{Backend, PluginRunEvent, PluginRunOutcome, PluginRunSpec, RunTarget},
    hf::HfModel,
    plugins::{InputKind, InputValue, PluginInfo},
    provision::ProvisionSpec,
    quant::GgufListing,
    tunnel::TunnelSpec,
    vm::RunStatus,
};
use gpui::{App, AppContext, Context};

use super::{AppState, InputSlot, Load, Op, ProvisionProgress, ops::key_of};
use crate::{
    i18n::{keys, tr_args},
    route::Route,
};

pub const HF_RESULTS: u32 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PluginView {
    #[default]
    List,
    Form,
    Run,
}

/// Hugging Face lookups for an `hf-model` input. Each lookup carries a generation; a result
/// for an older one is dropped, so results always match the latest query.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HfLookup {
    pub(crate) generation: u64,
    pub results: Load<Vec<HfModel>>,
    pub repo: Option<String>,
    pub files: Load<GgufListing>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunForm {
    pub plugin: String,
    /// Non-text values (number, bool, select) by input id.
    pub values: std::collections::BTreeMap<String, InputValue>,
    /// The fresh session's options (kept while an existing session is chosen).
    pub provision: ProvisionSpec,
    /// An existing session to run on instead.
    pub existing: Option<String>,
    /// Stop a session provisioned for this run when the run fails.
    pub stop_on_failure: bool,
    pub hf: HfLookup,
    pub error: Option<Error>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunPhase {
    /// Provisioning or running.
    Running,
    Ready(Box<PluginRunOutcome>),
    Failed(Error),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginRun {
    pub plugin: String,
    /// It provisions its own session.
    pub provisions: bool,
    /// That session is stopped if the run fails.
    pub stop_on_failure: bool,
    /// Provisioning progress (fresh sessions only).
    pub provision: ProvisionProgress,
    /// Session it runs on, once known.
    pub session: Option<String>,
    pub account: Option<AccountId>,
    pub status: Option<RunStatus>,
    pub log: RunLog,
    pub phase: RunPhase,
}

impl PluginRun {
    /// After a failure: whether the session this run started is gone (stopped by core), so
    /// nothing bills; `None` while running or when it ran on an existing session.
    pub fn session_stopped(&self) -> Option<bool> {
        match &self.phase {
            RunPhase::Failed(_) | RunPhase::Cancelled if self.provisions => {
                Some(matches!(self.phase, RunPhase::Cancelled) || self.stop_on_failure)
            }
            _ => None,
        }
    }
}

/// Lines of a run's log the app keeps, the newest ones. The whole log is a file on the runtime.
pub const RUN_LOG_LINES_KEPT: usize = 5_000;

/// The newest lines of a run's log (secrets already redacted by core).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunLog {
    lines: VecDeque<String>,
    dropped: usize,
}

impl RunLog {
    fn extend(&mut self, lines: impl IntoIterator<Item = String>) {
        self.lines.extend(lines);
        let extra = self.lines.len().saturating_sub(RUN_LOG_LINES_KEPT);
        self.lines.drain(..extra);
        self.dropped += extra;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line(&self, ix: usize) -> Option<&str> {
        self.lines.get(ix).map(String::as_str)
    }

    /// Lines no longer kept. A view that holds a place in the log moves it up by what this grew.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

impl AppState {
    pub fn plugin_view(&self) -> PluginView {
        self.plugin_view
    }

    pub fn run_form(&self) -> Option<&RunForm> {
        self.run_form.as_ref()
    }

    /// The current (or last) run.
    pub fn plugin_run(&self) -> Option<&PluginRun> {
        self.plugin_run.as_ref()
    }

    pub fn show_plugin_list(&mut self, cx: &mut Context<Self>) {
        self.plugin_view = PluginView::List;
        cx.notify();
    }

    pub fn show_plugin_run(&mut self, cx: &mut Context<Self>) {
        if self.plugin_run.is_some() {
            self.plugin_view = PluginView::Run;
            self.navigate(Route::Plugins, cx);
        }
    }

    pub fn open_run_form(&mut self, plugin: String, cx: &mut Context<Self>) {
        self.navigate(Route::Plugins, cx);
        let work_plugin = plugin.clone();
        self.run_backend(
            Op::RunTemplate,
            cx,
            move |backend, _| backend.plugin_run_template(&work_plugin),
            move |state, result, cx| {
                match result {
                    Ok(template) => state.fill_run_form(template, cx),
                    Err(error) => state.toast_error(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn fill_run_form(&mut self, template: PluginRunSpec, cx: &mut Context<Self>) {
        let Some(info) = self.plugin(&template.plugin).cloned() else {
            return;
        };
        let provision = match &template.target {
            RunTarget::Provision(spec) => spec.clone(),
            RunTarget::Existing { .. } => self.defaults().provision_spec(String::new()),
        };
        let mut values = std::collections::BTreeMap::new();
        for input in &info.manifest.inputs {
            let value = template.inputs.get(&input.id).cloned();
            match input.kind {
                InputKind::Text | InputKind::HfModel | InputKind::Number => {
                    let text = value.map(|v| v.to_env()).unwrap_or_default();
                    self.set_input(InputSlot::Field(input.id.clone()), text);
                }
                InputKind::Secret => {
                    self.set_input(InputSlot::Field(input.id.clone()), String::new())
                }
                InputKind::Bool | InputKind::Select => {
                    if let Some(value) = value {
                        values.insert(input.id.clone(), value);
                    }
                }
            }
        }
        self.set_input(InputSlot::RunSessionName, provision.name.clone());
        self.run_form = Some(RunForm {
            plugin: info.manifest.id.clone(),
            values,
            provision,
            existing: None,
            stop_on_failure: template.stop_on_failure,
            hf: HfLookup::default(),
            error: None,
        });
        self.plugin_view = PluginView::Form;
        cx.notify();
    }

    /// Edits the form's non-text values, target or provisioning options.
    pub fn edit_run_form(&mut self, edit: impl FnOnce(&mut RunForm), cx: &mut Context<Self>) {
        if let Some(form) = &mut self.run_form {
            edit(form);
            if !form.provision.accelerator.is_gpu() {
                form.provision.min_vram_mib = None;
            }
            form.error = None;
            cx.notify();
        }
    }

    pub fn close_run_form(&mut self, cx: &mut Context<Self>) {
        self.run_form = None;
        self.plugin_view = PluginView::List;
        cx.notify();
    }

    /// Searches Hugging Face; only the latest query's results are kept.
    pub fn search_hf(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(form) = &mut self.run_form else {
            return;
        };
        form.hf.generation += 1;
        form.hf.results = Load::Loading;
        let generation = form.hf.generation;
        self.run_latest(
            cx,
            move |backend, cancel| backend.hf_search(query.trim(), HF_RESULTS, cancel),
            move |state, result, cx| {
                if let Some(form) = &mut state.run_form
                    && form.hf.generation == generation
                {
                    form.hf.results = match result {
                        Ok(models) => Load::Ready(models),
                        Err(error) => Load::Failed(error),
                    };
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub fn choose_hf_repo(&mut self, field: String, repo: String, cx: &mut Context<Self>) {
        self.set_input(InputSlot::Field(field), repo.clone());
        self.load_hf_files(repo, cx);
    }

    /// Lists `repo`'s GGUF files (for the quantisation preview).
    pub fn load_hf_files(&mut self, repo: String, cx: &mut Context<Self>) {
        let repo = repo.trim().to_owned();
        let Some(form) = &mut self.run_form else {
            return;
        };
        if repo.is_empty()
            || (form.hf.repo.as_deref() == Some(&repo) && form.hf.files.ready().is_some())
        {
            return;
        }
        form.hf.repo = Some(repo.clone());
        form.hf.files = Load::Loading;
        let work_repo = repo.clone();
        self.run_backend(
            Op::HfFiles(key_of(&repo)),
            cx,
            move |backend, cancel| backend.hf_files(&work_repo, cancel),
            move |state, result, cx| {
                if let Some(form) = &mut state.run_form
                    && form.hf.repo.as_deref() == Some(&repo)
                {
                    form.hf.files = match result {
                        Ok(listing) => Load::Ready(listing),
                        Err(error) => Load::Failed(error),
                    };
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    /// Starts the run with the text fields' contents (`texts`: input id → text, secrets
    /// included; the caller clears the secret fields).
    pub fn start_run(
        &mut self,
        texts: Vec<(String, String)>,
        session_name: String,
        cx: &mut Context<Self>,
    ) {
        if self.ops.is_running(Op::PluginRun) {
            return;
        }
        let Some(form) = &self.run_form else {
            return;
        };
        let Some(info) = self.plugin(&form.plugin).cloned() else {
            return;
        };
        let mut inputs = form.values.clone();
        for (id, text) in texts {
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let numeric = info
                .manifest
                .input(&id)
                .is_some_and(|input| input.kind == InputKind::Number);
            let value = if numeric {
                match text.parse::<f64>() {
                    Ok(number) => InputValue::Number(number),
                    Err(_) => {
                        let error = Error::new(ErrorKind::InvalidInput)
                            .with("field", &id)
                            .with("value", text);
                        if let Some(form) = &mut self.run_form {
                            form.error = Some(error);
                        }
                        cx.notify();
                        return;
                    }
                }
            } else {
                InputValue::Text(text.to_owned())
            };
            inputs.insert(id, value);
        }
        if let Some(missing) = missing_required(&info, &inputs) {
            let error = Error::new(ErrorKind::InvalidInput).with("field", missing);
            if let Some(form) = &mut self.run_form {
                form.error = Some(error);
            }
            cx.notify();
            return;
        }
        let target = match &form.existing {
            Some(name) => RunTarget::Existing { name: name.clone() },
            None => {
                let mut spec = form.provision.clone();
                if !session_name.trim().is_empty() {
                    spec.name = session_name.trim().to_owned();
                }
                RunTarget::Provision(spec)
            }
        };
        let provisions = matches!(target, RunTarget::Provision(_));
        let spec = PluginRunSpec {
            plugin: form.plugin.clone(),
            inputs,
            target,
            stop_on_failure: form.stop_on_failure,
        };
        self.plugin_run = Some(PluginRun {
            plugin: form.plugin.clone(),
            provisions,
            stop_on_failure: form.stop_on_failure,
            provision: ProvisionProgress::default(),
            session: None,
            account: None,
            status: None,
            log: RunLog::default(),
            phase: RunPhase::Running,
        });
        self.plugin_view = PluginView::Run;
        self.run_backend_streaming(
            Op::PluginRun,
            cx,
            move |backend, events, cancel| backend.run_plugin(spec, events, cancel),
            |state, event, cx| {
                state.apply_run_event(event, cx);
                cx.notify();
            },
            |state, result, cx| {
                state.finish_run(result, cx);
                state.continue_stop_all(cx);
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn apply_run_event(&mut self, event: PluginRunEvent, cx: &App) {
        let at = self.backend_now();
        let Some(run) = &mut self.plugin_run else {
            return;
        };
        match event {
            PluginRunEvent::Provision(event) => run.provision.push(event, at),
            PluginRunEvent::Launched { session, .. } => run.session = Some(session),
            PluginRunEvent::Status(status) => run.status = Some(status),
            PluginRunEvent::Log { lines } => run.log.extend(lines),
            PluginRunEvent::LogSkipped { path } => {
                run.log
                    .extend([tr_args(cx, keys::RUN_LOG_SKIPPED, &[("path", &path)]).into()])
            }
        }
    }

    fn finish_run(
        &mut self,
        result: colab_playground_core::Result<PluginRunOutcome>,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = &mut self.plugin_run else {
            return;
        };
        let mut forwards = Vec::new();
        run.phase = match result {
            Ok(outcome) => {
                run.session = Some(outcome.session.clone());
                run.account = Some(outcome.account.clone());
                run.status = Some(outcome.status.clone());
                for endpoint in &outcome.endpoints {
                    forwards.push(TunnelSpec {
                        account: outcome.account.clone(),
                        session: outcome.session.clone(),
                        local_port: endpoint.remote_port,
                        remote_port: endpoint.remote_port,
                        health_path: endpoint.health.clone(),
                    });
                }
                RunPhase::Ready(Box::new(outcome))
            }
            Err(error) if error.is(ErrorKind::Cancelled) => RunPhase::Cancelled,
            Err(error) => RunPhase::Failed(error),
        };
        let active = self.accounts.ready().and_then(|view| view.active.clone());
        let Some(run) = &mut self.plugin_run else {
            return;
        };
        if run.account.is_none() {
            run.account = active;
        }
        let ready = matches!(run.phase, RunPhase::Ready(_));
        let plugin = run.plugin.clone();
        if ready {
            let name = self
                .plugin(&plugin)
                .map(|info| self.plugin_name(info))
                .unwrap_or(plugin);
            self.toast_done(tr_args(cx, keys::RUN_READY_TOAST, &[("name", &name)]), cx);
            self.count_ready(cx);
        }
        for spec in forwards {
            self.open_tunnel(spec, false, cx);
        }
        self.after_runtime_change(cx);
    }

    /// Asks the run to stop; a session it provisioned is stopped too.
    pub fn cancel_run(&mut self, cx: &mut Context<Self>) {
        self.ops.cancel(Op::PluginRun);
        cx.notify();
    }

    pub fn is_running_plugin(&self) -> bool {
        self.ops.is_running(Op::PluginRun)
    }

    /// Whether a run's cancel was asked and it is still winding down.
    pub fn is_cancelling_run(&self) -> bool {
        self.ops.is_cancelling(Op::PluginRun)
    }

    /// The runtime a run used, as listed (to stop it from the run view).
    pub fn run_runtime(&self) -> Option<colab_playground_core::backend::RuntimeRef> {
        let run = self.plugin_run.as_ref()?;
        let session = run.session.as_ref()?;
        self.runtimes()
            .ready()?
            .runtimes
            .iter()
            .find(|runtime| {
                runtime.id.name.as_ref() == Some(session)
                    && run
                        .account
                        .as_ref()
                        .is_none_or(|a| &runtime.id.account == a)
            })
            .map(|runtime| runtime.id.clone())
    }

    /// Runs `work` on the background executor outside the one-per-kind rule; `done` decides
    /// whether the result is still wanted (latest-wins lookups).
    pub(crate) fn run_latest<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(
            &dyn Backend,
            &colab_playground_core::CancelToken,
        ) -> colab_playground_core::Result<R>
        + Send
        + 'static,
        done: impl FnOnce(&mut AppState, colab_playground_core::Result<R>, &mut Context<AppState>)
        + 'static,
    ) {
        let backend: Arc<dyn Backend> = match &self.backend {
            Ok(backend) => backend.clone(),
            Err(error) => {
                let error = error.clone();
                done(self, Err(error), cx);
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let token = colab_playground_core::CancelToken::new();
            let result = cx
                .background_spawn(async move { work(&*backend, &token) })
                .await;
            this.update(cx, |state, cx| done(state, result, cx)).ok();
        })
        .detach();
    }
}

fn missing_required(
    info: &PluginInfo,
    inputs: &std::collections::BTreeMap<String, InputValue>,
) -> Option<String> {
    info.manifest
        .inputs
        .iter()
        .find(|input| {
            input.required
                && !matches!(inputs.get(&input.id), Some(InputValue::Text(t)) if !t.trim().is_empty())
                && !matches!(inputs.get(&input.id), Some(InputValue::Number(_) | InputValue::Bool(_)))
        })
        .map(|input| input.id.clone())
}
