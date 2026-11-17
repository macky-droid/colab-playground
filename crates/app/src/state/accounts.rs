//! Accounts: the list, signing in, switching and removing.
//!
//! A sign-in holds a `LoginFlow` from core. Dropping one rolls the CLI back to the previous
//! account and may block (the real one runs a helper), so a flow is only ever finished,
//! cancelled or dropped inside a background task, never on the UI thread.

use std::time::SystemTime;

use colab_playground_core::{
    Error, ErrorKind,
    accounts::{Account, AccountId, AccountsView},
    backend::{AccountUsageView, LoginFlow, SwitchImpact},
    host::ProbeReport,
};
use ely_gpui_component::{feedback::Toast, primitives::Severity};
use gpui::{AppContext, Context};

use super::{AppState, Load, Op};
use crate::i18n::{keys, tr, tr_args};

/// Where adding an account stands.
#[derive(Clone, Debug, PartialEq)]
pub enum AddPhase {
    /// Explains what happens; nothing has started.
    Intro,
    /// Asking the CLI for a sign-in page.
    Starting,
    /// The page is open; waiting for the pasted code.
    Code {
        /// The sign-in page.
        url: String,
    },
    /// Checking the code.
    Submitting {
        /// The sign-in page.
        url: String,
    },
    /// It failed; the previous account is active again and the flow has to start over.
    Failed {
        /// What went wrong.
        error: Error,
    },
}

/// The account dialog on show.
#[derive(Clone, Debug)]
pub enum AccountDialog {
    /// Adding a Google account.
    Add(AddPhase),
    /// Confirming a switch, with what it affects.
    Switch {
        /// The account to switch to.
        to: AccountId,
        /// Its email.
        email: Option<String>,
        /// What switching affects, and the backend's time when it was asked.
        impact: Load<(SwitchImpact, SystemTime)>,
        /// Why the switch failed.
        error: Option<Error>,
    },
    /// Confirming a removal.
    Remove {
        /// The account.
        id: AccountId,
        /// Its email.
        email: Option<String>,
        /// Why removing failed.
        error: Option<Error>,
    },
}

/// Where signing in stands, for the checklist, the switcher and the Accounts page.
#[derive(Clone, Debug, PartialEq)]
pub enum SignInStatus {
    /// Not known yet.
    Checking,
    /// The CLI has to be installed first.
    NeedsCli,
    /// The OS keyring cannot be reached.
    KeyringUnavailable(Error),
    /// Listing accounts failed otherwise.
    Failed(Error),
    /// No account is signed in.
    SignedOut,
    /// The CLI is signed in with an account that is not saved here (offer "use this account").
    Unadopted {
        /// Its email, when known.
        email: Option<String>,
    },
    /// An account is active.
    SignedIn {
        /// Its email, when known.
        email: Option<String>,
    },
}

/// Where signing in stands, from what is known. Pure, so tests can feed hand-built views
/// (the mock never reports an unadopted or recovered state).
pub fn sign_in_status(probe: &Load<ProbeReport>, accounts: &Load<AccountsView>) -> SignInStatus {
    match probe {
        Load::Idle | Load::Loading => return SignInStatus::Checking,
        Load::Ready(report) if report.colab.is_none() => return SignInStatus::NeedsCli,
        Load::Failed(error) if error.is(ErrorKind::NotInstalled) => {
            return SignInStatus::NeedsCli;
        }
        _ => {}
    }
    match accounts {
        Load::Idle | Load::Loading => SignInStatus::Checking,
        Load::Failed(error) if error.is(ErrorKind::KeyringUnavailable) => {
            SignInStatus::KeyringUnavailable(error.clone())
        }
        Load::Failed(error) if error.is(ErrorKind::NotInstalled) => SignInStatus::NeedsCli,
        Load::Failed(error) => SignInStatus::Failed(error.clone()),
        Load::Ready(view) => match view.active_account() {
            Some(account) => SignInStatus::SignedIn {
                email: account.email.clone(),
            },
            None if view.unadopted || (view.has_token && view.active.is_none()) => {
                SignInStatus::Unadopted {
                    email: view.active_email.clone(),
                }
            }
            None => SignInStatus::SignedOut,
        },
    }
}

/// Why an account cannot be removed right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveBlock {
    /// It is the active account.
    Active,
    /// Runtimes run on it.
    Running,
}

/// One row of the Accounts page.
#[derive(Clone, Debug)]
pub struct AccountRow {
    /// The account.
    pub account: Account,
    /// It is the active one.
    pub active: bool,
    /// Its usage, when fetched.
    pub usage: Option<AccountUsageView>,
    /// Why Remove is unavailable.
    pub remove_block: Option<RemoveBlock>,
}

/// The row for `account`.
pub fn account_row(
    account: &Account,
    active: Option<&AccountId>,
    usage: Option<&AccountUsageView>,
) -> AccountRow {
    let is_active = active == Some(&account.id);
    let running = usage
        .and_then(|usage| usage.snapshot)
        .or(account.usage)
        .is_some_and(|snapshot| snapshot.assignments > 0);
    let remove_block = if is_active {
        Some(RemoveBlock::Active)
    } else if running {
        Some(RemoveBlock::Running)
    } else {
        None
    };
    AccountRow {
        account: account.clone(),
        active: is_active,
        usage: usage.cloned(),
        remove_block,
    }
}

impl AppState {
    /// The saved accounts and the CLI slot.
    pub fn accounts(&self) -> &Load<AccountsView> {
        &self.accounts
    }

    /// Where signing in stands.
    pub fn sign_in_status(&self) -> SignInStatus {
        sign_in_status(&self.probe, &self.accounts)
    }

    /// The rows of the Accounts page, active first.
    pub fn account_rows(&self) -> Vec<AccountRow> {
        let Some(view) = self.accounts.ready() else {
            return Vec::new();
        };
        let mut rows: Vec<AccountRow> = view
            .accounts
            .iter()
            .map(|account| account_row(account, view.active.as_ref(), self.usage_for(&account.id)))
            .collect();
        rows.sort_by_key(|row| !row.active);
        rows
    }

    /// The email of a saved account.
    pub fn email_of(&self, id: &AccountId) -> Option<String> {
        self.accounts
            .ready()?
            .accounts
            .iter()
            .find(|account| &account.id == id)?
            .email
            .clone()
    }

    /// Whether an interrupted sign-in was rolled back (shown until dismissed).
    pub fn recovered_notice(&self) -> bool {
        self.recovered_notice
    }

    /// Hides the rolled-back notice.
    pub fn dismiss_recovered_notice(&mut self, cx: &mut Context<Self>) {
        self.recovered_notice = false;
        cx.notify();
    }

    /// The account dialog on show.
    pub fn dialog(&self) -> Option<&AccountDialog> {
        self.dialog.as_ref()
    }

    /// Lists accounts, then fetches usage.
    pub fn load_accounts(&mut self, cx: &mut Context<Self>) {
        if !self.cli_installed() {
            self.accounts = Load::Failed(Self::not_installed());
            cx.notify();
            return;
        }
        self.accounts.begin();
        self.run_backend(
            Op::Accounts,
            cx,
            |backend, cancel| backend.accounts(cancel),
            |state, result, cx| match result {
                Ok(view) => {
                    if view.recovered_pending_add {
                        state.recovered_notice = true;
                    }
                    let has_accounts = !view.accounts.is_empty();
                    state.accounts = Load::Ready(view);
                    if has_accounts {
                        state.refresh_usage(true, cx);
                        state.load_runtimes(cx);
                    }
                }
                Err(error) => state.accounts = Load::Failed(error),
            },
        );
    }

    /// Saves the account the CLI is signed in with.
    pub fn adopt_account(&mut self, cx: &mut Context<Self>) {
        self.run_backend(
            Op::Adopt,
            cx,
            |backend, cancel| backend.adopt_active_account(cancel),
            |state, result, cx| match result {
                Ok(account) => {
                    let email = account.email.clone().unwrap_or_default();
                    state.toast_done(tr_args(cx, keys::ADOPT_DONE, &[("email", &email)]), cx);
                    state.load_accounts(cx);
                }
                Err(error) => state.accounts = Load::Failed(error),
            },
        );
    }

    /// Opens the add-account dialog (nothing starts until the user continues).
    pub fn open_add_account(&mut self, cx: &mut Context<Self>) {
        if let Some(reason) = self.account_change_block() {
            let toast = Toast::new(tr(cx, reason)).severity(Severity::Warning);
            self.toast(toast, cx);
            return;
        }
        self.overlay = None;
        if !matches!(self.dialog, Some(AccountDialog::Add(_))) {
            self.dialog = Some(AccountDialog::Add(AddPhase::Intro));
            cx.notify();
        }
    }

    /// Asks the CLI for a sign-in page and opens it in the browser.
    pub fn begin_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.login_flow.is_some() || self.is_busy(Op::Login) {
            return;
        }
        self.dialog = Some(AccountDialog::Add(AddPhase::Starting));
        self.run_backend(
            Op::Login,
            cx,
            |backend, cancel| backend.begin_add_account(cancel),
            |state, result, cx| match result {
                Ok(flow) => {
                    if !matches!(state.dialog, Some(AccountDialog::Add(AddPhase::Starting))) {
                        log::info!("accounts: the dialog closed while starting; rolling back");
                        state.roll_back(flow, cx);
                        return;
                    }
                    let url = flow.auth_url().to_owned();
                    state.login_flow = Some(flow);
                    state.dialog = Some(AccountDialog::Add(AddPhase::Code { url: url.clone() }));
                    cx.open_url(&url);
                }
                Err(error) if error.is(ErrorKind::Cancelled) => {}
                Err(error) => {
                    if matches!(state.dialog, Some(AccountDialog::Add(_))) {
                        state.dialog = Some(AccountDialog::Add(AddPhase::Failed { error }));
                    }
                }
            },
        );
    }

    /// Sends the pasted code (never logged).
    pub fn submit_code(&mut self, code: String, cx: &mut Context<Self>) {
        let Some(Some(AccountDialog::Add(AddPhase::Code { url }))) = Some(self.dialog.clone())
        else {
            return;
        };
        let Some(flow) = self.login_flow.take() else {
            return;
        };
        self.dialog = Some(AccountDialog::Add(AddPhase::Submitting { url }));
        self.run_backend(
            Op::Login,
            cx,
            move |_, cancel| flow.submit_code(&code, cancel),
            |state, result, cx| match result {
                Ok(account) => {
                    if matches!(state.dialog, Some(AccountDialog::Add(_))) {
                        state.dialog = None;
                    }
                    let email = account.email.clone().unwrap_or_default();
                    state.toast_done(tr_args(cx, keys::ADD_DONE, &[("email", &email)]), cx);
                    state.load_accounts(cx);
                }
                Err(error) => {
                    if matches!(state.dialog, Some(AccountDialog::Add(_))) {
                        state.dialog = Some(AccountDialog::Add(AddPhase::Failed { error }));
                    }
                    state.load_accounts(cx);
                }
            },
        );
    }

    /// Closes the add-account dialog; an open sign-in is rolled back.
    pub fn close_add_account(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.dialog, Some(AccountDialog::Add(_))) {
            return;
        }
        self.dialog = None;
        if self.ops.is_running(Op::Login) {
            self.ops.cancel(Op::Login);
        }
        if let Some(flow) = self.login_flow.take() {
            self.roll_back(flow, cx);
        }
        cx.notify();
    }

    /// Cancels `flow` on the background executor (the previous account comes back), then
    /// lists accounts again.
    fn roll_back(&mut self, flow: Box<dyn LoginFlow>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let cancelled = cx.background_spawn(async move { flow.cancel() }).await;
            if let Err(error) = &cancelled {
                log::error!("accounts: rolling back the sign-in failed: {error}");
            }
            this.update(cx, |state, cx| state.load_accounts(cx)).ok();
        })
        .detach();
    }

    /// Opens the switch confirmation and asks what switching affects.
    pub fn request_switch(&mut self, to: AccountId, cx: &mut Context<Self>) {
        if let Some(reason) = self.account_change_block() {
            let toast = Toast::new(tr(cx, reason)).severity(Severity::Warning);
            self.toast(toast, cx);
            return;
        }
        self.overlay = None;
        let email = self.email_of(&to);
        self.dialog = Some(AccountDialog::Switch {
            to: to.clone(),
            email,
            impact: Load::Loading,
            error: None,
        });
        self.run_backend(
            Op::SwitchImpact,
            cx,
            move |backend, _| {
                let impact = backend.switch_impact(&to)?;
                Ok((impact, backend.now()))
            },
            |state, result, cx| {
                if let Some(AccountDialog::Switch { impact, to, .. }) = &mut state.dialog {
                    *impact = match result {
                        Ok(found) if &found.0.to == to => Load::Ready(found),
                        Ok(_) => return,
                        Err(error) => Load::Failed(error),
                    };
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    /// Switches to the account the open dialog names.
    pub fn confirm_switch(&mut self, cx: &mut Context<Self>) {
        let Some(AccountDialog::Switch { to, .. }) = &self.dialog else {
            return;
        };
        let to = to.clone();
        self.run_backend(
            Op::Switch,
            cx,
            move |backend, cancel| backend.switch_account(&to, cancel),
            |state, result, cx| match result {
                Ok(view) => {
                    state.dialog = None;
                    let email = view.active_email.clone().unwrap_or_default();
                    state.accounts = Load::Ready(view);
                    state.toast_done(tr_args(cx, keys::SWITCH_DONE, &[("email", &email)]), cx);
                    state.refresh_usage(true, cx);
                    state.load_runtimes(cx);
                }
                Err(error) => {
                    if let Some(AccountDialog::Switch { error: shown, .. }) = &mut state.dialog {
                        *shown = Some(error);
                    }
                }
            },
        );
    }

    /// Opens the remove confirmation.
    pub fn request_remove(&mut self, id: AccountId, cx: &mut Context<Self>) {
        let email = self.email_of(&id);
        self.dialog = Some(AccountDialog::Remove {
            id,
            email,
            error: None,
        });
        cx.notify();
    }

    /// Removes the account the open dialog names.
    pub fn confirm_remove(&mut self, cx: &mut Context<Self>) {
        let Some(AccountDialog::Remove { id, email, .. }) = &self.dialog else {
            return;
        };
        let (id, email) = (id.clone(), email.clone().unwrap_or_default());
        self.run_backend(
            Op::Remove,
            cx,
            move |backend, cancel| backend.remove_account(&id, cancel),
            move |state, result, cx| match result {
                Ok(()) => {
                    state.dialog = None;
                    state.toast_done(tr_args(cx, keys::REMOVE_DONE, &[("email", &email)]), cx);
                    state.load_accounts(cx);
                }
                Err(error) => {
                    if let Some(AccountDialog::Remove { error: shown, .. }) = &mut state.dialog {
                        *shown = Some(error);
                    }
                }
            },
        );
    }

    /// Closes the switch or remove dialog (the add dialog has [`Self::close_add_account`]).
    pub fn close_dialog(&mut self, cx: &mut Context<Self>) {
        match self.dialog {
            Some(AccountDialog::Add(_)) => self.close_add_account(cx),
            Some(_) => {
                self.dialog = None;
                cx.notify();
            }
            None => {}
        }
    }

    /// The label of the active account, when one is signed in.
    pub fn active_email(&self) -> Option<String> {
        match self.sign_in_status() {
            SignInStatus::SignedIn { email } => email,
            _ => None,
        }
    }
}
