//! The environment: probe, CLI install and the first-run checklist.

use std::rc::Rc;

use colab_playground_core::{
    Error, ErrorKind,
    accounts::AccountsView,
    backend::InstallMethod,
    host::{Line, ProbeReport},
};
use gpui::{Context, ScrollHandle};

use super::{AppState, Load, Op};
use crate::platform::EnvironmentKind;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum InstallPhase {
    #[default]
    Idle,
    /// Running (its log streams in).
    Running,
    /// Finished; the environment is checked again.
    Done,
    Cancelled,
    Failed(Error),
}

#[derive(Clone, Debug, Default)]
pub struct InstallState {
    pub phase: InstallPhase,
    pub lines: Rc<Vec<Line>>,
    /// The method the user picked; `None` = the recommended one.
    pub method: Option<InstallMethod>,
    /// Keeps the log scrolled to its end.
    pub scroll: ScrollHandle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StepState {
    Checking,
    Done,
    /// The user has something to do here.
    Todo,
    /// Waiting for an earlier step.
    Waiting,
    Failed(Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentStep {
    /// WSL with a distribution (Windows).
    Wsl,
    /// `bash` (and `ssh`) on the host (macOS, Linux).
    ShellTools,
    Cli,
    /// A saved, active Google account.
    SignIn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub step: EnvironmentStep,
    pub state: StepState,
}

/// The first-run checklist, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Checklist {
    pub steps: Vec<Step>,
}

impl Checklist {
    pub fn complete(&self) -> bool {
        self.steps.iter().all(|step| step.state == StepState::Done)
    }

    /// The state of `step`, if the checklist has it.
    pub fn state(&self, step: EnvironmentStep) -> Option<&StepState> {
        self.steps
            .iter()
            .find(|each| each.step == step)
            .map(|each| &each.state)
    }
}

/// The checklist for what is known: `environment` says whether the first step is WSL (Windows)
/// or shell tools. A pure function, so every platform's variant is tested on any host.
pub fn checklist(
    environment: EnvironmentKind,
    probe: &Load<ProbeReport>,
    accounts: &Load<AccountsView>,
) -> Checklist {
    let first = match environment {
        EnvironmentKind::Wsl => EnvironmentStep::Wsl,
        EnvironmentKind::ShellTools => EnvironmentStep::ShellTools,
    };
    let checking = matches!(probe, Load::Idle | Load::Loading);
    let (env_state, cli_state) = match probe {
        Load::Idle | Load::Loading => (StepState::Checking, StepState::Waiting),
        Load::Failed(error) if error.is(ErrorKind::NotInstalled) => {
            (StepState::Done, StepState::Todo)
        }
        Load::Failed(error) => (StepState::Failed(error.clone()), StepState::Waiting),
        Load::Ready(report) => {
            let env_ok = match environment {
                EnvironmentKind::Wsl => report.wsl_installed && !report.wsl_distros.is_empty(),
                EnvironmentKind::ShellTools => report.bash.is_some(),
            };
            match (env_ok, report.colab.is_some()) {
                (false, _) => (StepState::Todo, StepState::Waiting),
                (true, false) => (StepState::Done, StepState::Todo),
                (true, true) => (StepState::Done, StepState::Done),
            }
        }
    };
    let sign_in = if checking || cli_state != StepState::Done {
        StepState::Waiting
    } else {
        match accounts {
            Load::Idle | Load::Loading => StepState::Checking,
            Load::Failed(error) => StepState::Failed(error.clone()),
            Load::Ready(view) if view.active_account().is_some() => StepState::Done,
            Load::Ready(_) => StepState::Todo,
        }
    };
    Checklist {
        steps: vec![
            Step {
                step: first,
                state: env_state,
            },
            Step {
                step: EnvironmentStep::Cli,
                state: cli_state,
            },
            Step {
                step: EnvironmentStep::SignIn,
                state: sign_in,
            },
        ],
    }
}

/// Ways to install the CLI on this host, recommended first: `uv` when present, otherwise its
/// installer; `pip --user` wherever `python3` exists.
pub fn install_methods(report: Option<&ProbeReport>) -> Vec<InstallMethod> {
    let Some(report) = report else {
        return vec![InstallMethod::InstallUvThenCli];
    };
    let mut methods = vec![if report.uv.is_some() {
        InstallMethod::Uv
    } else {
        InstallMethod::InstallUvThenCli
    }];
    if report.python3.is_some() {
        methods.push(InstallMethod::PipUser);
    }
    methods
}

impl AppState {
    pub fn probe(&self) -> &Load<ProbeReport> {
        &self.probe
    }

    pub fn install(&self) -> &InstallState {
        &self.install
    }

    pub fn cli_installed(&self) -> bool {
        self.probe
            .ready()
            .is_some_and(|report| report.colab.is_some())
    }

    pub fn checklist(&self) -> Checklist {
        checklist(
            crate::platform::environment_kind(),
            &self.probe,
            &self.accounts,
        )
    }

    /// Probes the host, then (with the CLI present) lists accounts and fetches usage. Plugins
    /// are listed either way: the list is kept on this computer.
    pub fn check_environment(&mut self, cx: &mut Context<Self>) {
        self.probe.begin();
        self.run_backend(
            Op::Environment,
            cx,
            |backend, cancel| backend.probe(cancel),
            |state, result, cx| {
                state.probe = match result {
                    Ok(report) => Load::Ready(report),
                    Err(error) => Load::Failed(error),
                };
                if state.cli_installed() {
                    state.load_accounts(cx);
                } else {
                    state.accounts = Load::Idle;
                }
                state.load_plugins(cx);
            },
        );
        cx.notify();
    }

    pub fn set_install_method(&mut self, method: InstallMethod, cx: &mut Context<Self>) {
        self.install.method = Some(method);
        cx.notify();
    }

    /// The method an install would use now.
    pub fn chosen_install_method(&self) -> InstallMethod {
        let offered = install_methods(self.probe.ready());
        match self.install.method {
            Some(method) if offered.contains(&method) => method,
            _ => offered[0],
        }
    }

    /// Installs the CLI with the chosen method, streaming its log; checks the environment
    /// again when it succeeds.
    pub fn start_install(&mut self, cx: &mut Context<Self>) {
        if self.is_busy(Op::Install) {
            return;
        }
        let method = self.chosen_install_method();
        log::info!("install: starting with {method:?}");
        self.install.phase = InstallPhase::Running;
        self.install.lines = Rc::new(Vec::new());
        self.run_backend_streaming(
            Op::Install,
            cx,
            move |backend, log, cancel| backend.install_cli(method, log, cancel),
            |state, line: Line, _| {
                Rc::make_mut(&mut state.install.lines).push(line);
                state.install.scroll.scroll_to_bottom();
            },
            |state, result, cx| {
                state.install.phase = match result {
                    Ok(()) => InstallPhase::Done,
                    Err(error) if error.is(ErrorKind::Cancelled) => InstallPhase::Cancelled,
                    Err(error) => InstallPhase::Failed(error),
                };
                if state.install.phase == InstallPhase::Done {
                    state.check_environment(cx);
                }
            },
        );
    }

    pub fn cancel_install(&mut self, cx: &mut Context<Self>) {
        self.ops.cancel(Op::Install);
        cx.notify();
    }
}
