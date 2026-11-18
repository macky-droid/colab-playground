//! Views. The root view draws the title strip, the shell and the account dialogs; pages are
//! functions of the app state (plus the few text fields the root view owns).

pub mod dialogs;
pub mod duration_field;
pub mod error_view;
pub mod overlays;
pub mod pages;
pub mod progress;
pub mod provision_form;
pub mod root;
pub mod sidebar;
pub mod titlebar;

use ely_gpui_component::forms::TextInput;
use gpui::{Div, Entity, InteractiveElement, IntoElement, ParentElement, div};

/// Wraps `element` in a plain `div` that carries `selector` for
/// `VisualTestContext::debug_bounds` in test builds (gpui records selectors only with its
/// `test-support` feature in debug builds; elsewhere this is a bare `div`). Use it around Ely
/// components that do not expose a selector of their own (`NavItem`, `Button`, `Select`, ...)
/// so headless tests can find and click them.
pub fn tagged(selector: impl Into<String>, element: impl IntoElement) -> Div {
    let selector = selector.into();
    div().debug_selector(move || selector).child(element)
}

/// Text fields the root view owns (a `TextInput` needs a window to be made). State fills
/// them through `AppState::set_input`; the root view applies that before drawing.
#[derive(Clone)]
pub struct Inputs {
    pub code: Entity<TextInput>,
    /// Settings: path of `colab`.
    pub colab_path: Entity<TextInput>,
    /// Settings: path of its Python interpreter.
    pub colab_python: Entity<TextInput>,
    /// New session: its name.
    pub session_name: Entity<TextInput>,
    /// Run form: the fresh session's name.
    pub run_session_name: Entity<TextInput>,
    /// Forward: the optional health path.
    pub health: Entity<TextInput>,
    /// Forward: the port on the runtime (digits only).
    pub forward_remote: Entity<TextInput>,
    /// Forward: the port on this computer (digits only).
    pub forward_local: Entity<TextInput>,
    /// Plugins: the folder or Git URL to install from.
    pub plugin_source: Entity<TextInput>,
    /// Plugins: the Hugging Face search query.
    pub hf_query: Entity<TextInput>,
    /// Plugins: the Hugging Face token (masked; cleared once saved).
    pub hf_token: Entity<TextInput>,
    /// Run form: text, secret and model fields by input id (made when a form opens).
    pub fields: std::collections::BTreeMap<String, Entity<TextInput>>,
}

impl Inputs {
    /// The run-form field for input `id`.
    pub fn field(&self, id: &str) -> Option<Entity<TextInput>> {
        self.fields.get(id).cloned()
    }

    pub fn slot(&self, slot: &crate::state::InputSlot) -> Option<Entity<TextInput>> {
        use crate::state::InputSlot;
        match slot {
            InputSlot::SessionName => Some(self.session_name.clone()),
            InputSlot::RunSessionName => Some(self.run_session_name.clone()),
            InputSlot::PluginSource => Some(self.plugin_source.clone()),
            InputSlot::ForwardRemote => Some(self.forward_remote.clone()),
            InputSlot::ForwardLocal => Some(self.forward_local.clone()),
            InputSlot::Health => Some(self.health.clone()),
            InputSlot::Field(id) => self.field(id),
        }
    }
}
