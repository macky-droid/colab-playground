//! New session: the form (pre-filled from `Defaults`, every value overridable for this one
//! session) and the live provisioning progress. Cancel is always available and leaves no
//! billed runtime behind (core releases any VM it allocated).
//!
//! The same progress model serves a plugin run that provisions its own session.

use colab_playground_core::{
    Error, ErrorKind,
    provision::{ProvisionEvent, ProvisionOutcome, ProvisionSpec},
};
use gpui::{Context, ScrollHandle};

use super::{AppState, InputSlot, Op, Overlay};
use crate::i18n::{keys, tr_args};

#[derive(Clone, Debug, PartialEq)]
pub enum ProvisionPhase {
    Editing,
    /// Asking Colab for a runtime (see the events).
    Running,
    Ready(ProvisionOutcome),
    Failed(Error),
    /// Cancelled; nothing is left running.
    Cancelled,
}

/// What provisioning reported so far, folded into what the progress view shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProvisionProgress {
    /// Every event, in order.
    pub events: Vec<ProvisionEvent>,
}

impl ProvisionProgress {
    /// `(attempt, of)` of the latest attempt.
    pub fn attempt(&self) -> Option<(u32, u32)> {
        self.events.iter().rev().find_map(|event| match event {
            ProvisionEvent::Allocating {
                attempt,
                max_attempts,
            } => Some((*attempt, *max_attempts)),
            _ => None,
        })
    }

    pub fn last(&self) -> Option<&ProvisionEvent> {
        self.events.last()
    }
}

#[derive(Clone, Debug)]
pub struct NewSession {
    pub spec: ProvisionSpec,
    pub phase: ProvisionPhase,
    pub progress: ProvisionProgress,
    /// New on each opening, so the form opens at the top.
    pub scroll: ScrollHandle,
}

impl AppState {
    /// The new-session dialog's state, while one exists (also while it is hidden and
    /// provisioning continues).
    pub fn new_session(&self) -> Option<&NewSession> {
        self.new_session.as_ref()
    }

    /// Whether a runtime is being provisioned (by the dialog or a plugin run).
    pub fn is_provisioning(&self) -> bool {
        self.ops.is_running(Op::Provision)
    }

    pub fn open_new_session(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.new_session
            && session.phase == ProvisionPhase::Running
        {
            self.overlay = Some(Overlay::NewSession);
            cx.notify();
            return;
        }
        let defaults = self.defaults().clone();
        let spec = defaults.provision_spec(String::new());
        self.new_session = Some(NewSession {
            spec,
            phase: ProvisionPhase::Editing,
            progress: ProvisionProgress::default(),
            scroll: ScrollHandle::new(),
        });
        self.overlay = Some(Overlay::NewSession);
        self.run_backend(
            Op::SessionName,
            cx,
            |backend, _| Ok(backend.new_session_name(None)),
            |state, result, cx| {
                if let Ok(name) = result
                    && let Some(session) = &mut state.new_session
                    && session.phase == ProvisionPhase::Editing
                {
                    session.spec.name = name.clone();
                    state.set_input(InputSlot::SessionName, name);
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub fn edit_new_session(
        &mut self,
        edit: impl FnOnce(&mut ProvisionSpec),
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = &mut self.new_session
            && session.phase == ProvisionPhase::Editing
        {
            edit(&mut session.spec);
            if !session.spec.accelerator.is_gpu() {
                session.spec.min_vram_mib = None;
            }
            cx.notify();
        }
    }

    pub fn start_provision(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(session) = &mut self.new_session else {
            return;
        };
        if session.phase != ProvisionPhase::Editing
            && !matches!(
                session.phase,
                ProvisionPhase::Failed(_) | ProvisionPhase::Cancelled
            )
        {
            return;
        }
        let name = name.trim().to_owned();
        if !name.is_empty() {
            session.spec.name = name;
        }
        if let Err(error) = session.spec.validate() {
            session.phase = ProvisionPhase::Failed(error);
            cx.notify();
            return;
        }
        session.phase = ProvisionPhase::Running;
        session.progress = ProvisionProgress::default();
        let spec = session.spec.clone();
        self.run_backend_streaming(
            Op::Provision,
            cx,
            move |backend, events, cancel| backend.provision(spec, events, cancel),
            |state, event, cx| {
                if let Some(session) = &mut state.new_session {
                    session.progress.events.push(event);
                    cx.notify();
                }
            },
            |state, result, cx| {
                if let Ok(outcome) = &result {
                    let name = outcome.name.clone();
                    state.toast_done(
                        tr_args(cx, keys::PROVISION_READY_TOAST, &[("name", &name)]),
                        cx,
                    );
                }
                let Some(session) = &mut state.new_session else {
                    return;
                };
                session.phase = match result {
                    Ok(outcome) => ProvisionPhase::Ready(outcome),
                    Err(error) if error.is(ErrorKind::Cancelled) => ProvisionPhase::Cancelled,
                    Err(error) => ProvisionPhase::Failed(error),
                };
                state.after_runtime_change(cx);
                state.continue_stop_all(cx);
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Asks provisioning to stop; it releases any VM it allocated before returning.
    pub fn cancel_provision(&mut self, cx: &mut Context<Self>) {
        self.ops.cancel(Op::Provision);
        cx.notify();
    }

    /// Whether a cancel was asked and provisioning is still winding down.
    pub fn is_cancelling_provision(&self) -> bool {
        self.ops.is_cancelling(Op::Provision)
    }

    /// Closes the dialog. While provisioning runs it only hides (the Sessions page shows the
    /// progress with Cancel); otherwise the form is dropped.
    pub fn close_new_session(&mut self, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::NewSession)) {
            self.overlay = None;
        }
        let running = self
            .new_session
            .as_ref()
            .is_some_and(|session| session.phase == ProvisionPhase::Running);
        if !running {
            self.new_session = None;
        }
        cx.notify();
    }

    /// Back to the form after a failure or cancel, with the same values.
    pub fn edit_again(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &mut self.new_session
            && matches!(
                session.phase,
                ProvisionPhase::Failed(_) | ProvisionPhase::Cancelled
            )
        {
            session.phase = ProvisionPhase::Editing;
            session.progress = ProvisionProgress::default();
            let name = session.spec.name.clone();
            self.set_input(InputSlot::SessionName, name);
            cx.notify();
        }
    }
}

/// The session cap on the active account, when it is reached.
#[derive(Clone, Debug, PartialEq)]
pub struct LimitReached {
    /// `max_concurrent_sessions` (this app's own setting).
    pub limit: u32,
    pub email: Option<String>,
    /// Runtimes that count: those this app can address by name (it started them).
    pub counted: Vec<String>,
}

impl AppState {
    /// The session cap, checked before anything is asked of Colab (core still enforces it).
    /// Like core, it counts named runtimes on the active account; runtimes opened elsewhere do
    /// not count.
    pub fn session_limit_reached(&self) -> Option<LimitReached> {
        let view = self.runtimes().ready()?;
        let limit = self.defaults().max_concurrent_sessions.max(1);
        let counted: Vec<String> = view
            .runtimes
            .iter()
            .filter(|r| r.on_active_account)
            .filter_map(|r| r.id.name.clone())
            .collect();
        (counted.len() as u32 >= limit).then(|| LimitReached {
            limit,
            email: self.active_email(),
            counted,
        })
    }

    /// Opens Settings with the session limit called out at the top.
    pub fn open_limit_setting(&mut self, cx: &mut Context<Self>) {
        self.close_overlay(cx);
        self.new_session = None;
        self.navigate(crate::route::Route::Settings, cx);
        self.settings_focus = true;
        cx.notify();
    }
}
