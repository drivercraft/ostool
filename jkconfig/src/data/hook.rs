use std::sync::Arc;

use crate::data::{app_data::AppState, path::ElementPath, types::ElementType};

/// Result of invoking a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFlow {
    ContinueDefault,
    Consumed,
}

/// Severity used by hook-triggered messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// One selectable option rendered by jkconfig.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOption {
    pub value: String,
    pub label: String,
    pub detail: Option<String>,
    pub disabled: bool,
}

impl HookOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: None,
            disabled: false,
        }
    }
}

pub type HookApplySingle =
    Arc<dyn for<'a> Fn(&mut HookMutation<'a>, String) -> anyhow::Result<()> + Send + Sync>;
pub type HookApplyMulti =
    Arc<dyn for<'a> Fn(&mut HookMutation<'a>, Vec<String>) -> anyhow::Result<()> + Send + Sync>;

#[derive(Clone)]
pub enum SingleSelectBinding {
    SetCurrentString,
    SetString { path: ElementPath },
    SetEnumVariant { path: ElementPath },
    Custom(HookApplySingle),
}

#[derive(Clone)]
pub enum MultiSelectBinding {
    SetCurrentStringArray,
    SetStringArray { path: ElementPath },
    Custom(HookApplyMulti),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Text,
    Integer,
    Number,
    Path,
    Command,
}

#[derive(Clone)]
pub enum InputBinding {
    SetCurrentString,
    SetString { path: ElementPath },
    SetOptionalString { path: ElementPath },
    SetInteger { path: ElementPath },
    SetOptionalInteger { path: ElementPath },
    SetNumber { path: ElementPath },
    SetOptionalNumber { path: ElementPath },
    Custom(HookApplySingle),
}

#[derive(Clone)]
pub struct SingleSelectSpec {
    pub title: String,
    pub help: Option<String>,
    pub options: Vec<HookOption>,
    pub initial: Option<String>,
    pub allow_clear: bool,
    pub binding: SingleSelectBinding,
}

#[derive(Clone)]
pub struct MultiSelectSpec {
    pub title: String,
    pub help: Option<String>,
    pub options: Vec<HookOption>,
    pub selected: Vec<String>,
    pub min_selected: Option<usize>,
    pub max_selected: Option<usize>,
    pub binding: MultiSelectBinding,
}

#[derive(Clone)]
pub struct InputPageSpec {
    pub title: String,
    pub label: String,
    pub help: Option<String>,
    pub initial: Option<String>,
    pub placeholder: Option<String>,
    pub kind: InputKind,
    pub allow_empty: bool,
    pub min_inner_width: u16,
    pub binding: InputBinding,
}

/// Hook registration for a specific element path.
#[derive(Clone)]
pub struct ElementHook {
    pub path: ElementPath,
    pub callback: HookCallback,
}

impl ElementHook {
    pub fn new(path: impl Into<ElementPath>, callback: HookCallback) -> Self {
        Self {
            path: path.into(),
            callback,
        }
    }
}

/// Public hook callback signature.
pub type HookCallback = Arc<
    dyn for<'a> Fn(&mut HookContext<'a>, &ElementPath) -> anyhow::Result<HookFlow> + Send + Sync,
>;

#[derive(Debug, Clone)]
pub(crate) struct PendingMessage {
    pub level: MessageLevel,
    pub text: String,
}

#[derive(Clone)]
pub(crate) enum PendingPresentation {
    SingleSelect {
        path: ElementPath,
        spec: SingleSelectSpec,
    },
    MultiSelect {
        path: ElementPath,
        spec: MultiSelectSpec,
    },
    Input {
        path: ElementPath,
        spec: InputPageSpec,
    },
}

/// Controlled mutator exposed to hook callbacks and modal bindings.
pub struct HookMutation<'a> {
    app: &'a mut AppState,
}

impl<'a> HookMutation<'a> {
    pub(crate) fn new(app: &'a mut AppState) -> Self {
        Self { app }
    }

    pub fn set_string(
        &mut self,
        path: impl Into<ElementPath>,
        value: String,
    ) -> anyhow::Result<()> {
        self.app.set_string(path, value)
    }

    pub fn set_optional_string(
        &mut self,
        path: impl Into<ElementPath>,
        value: Option<String>,
    ) -> anyhow::Result<()> {
        self.app.set_optional_string(path, value)
    }

    pub fn set_string_array(
        &mut self,
        path: impl Into<ElementPath>,
        values: Vec<String>,
    ) -> anyhow::Result<()> {
        self.app.set_string_array(path, values)
    }

    pub fn set_integer(&mut self, path: impl Into<ElementPath>, value: i64) -> anyhow::Result<()> {
        self.app.set_integer(path, value)
    }

    pub fn set_optional_integer(
        &mut self,
        path: impl Into<ElementPath>,
        value: Option<i64>,
    ) -> anyhow::Result<()> {
        self.app.set_optional_integer(path, value)
    }

    pub fn set_number(&mut self, path: impl Into<ElementPath>, value: f64) -> anyhow::Result<()> {
        self.app.set_number(path, value)
    }

    pub fn set_optional_number(
        &mut self,
        path: impl Into<ElementPath>,
        value: Option<f64>,
    ) -> anyhow::Result<()> {
        self.app.set_optional_number(path, value)
    }

    pub fn set_enum_variant(
        &mut self,
        path: impl Into<ElementPath>,
        variant: &str,
    ) -> anyhow::Result<()> {
        self.app.set_enum_variant(path, variant)
    }

    pub fn clear(&mut self, path: impl Into<ElementPath>) -> anyhow::Result<()> {
        self.app.clear_optional(path)
    }
}

/// Read state and request a built-in modal page from a hook.
pub struct HookContext<'a> {
    app: &'a mut AppState,
    path: ElementPath,
    pending_presentation: Option<PendingPresentation>,
    pending_messages: Vec<PendingMessage>,
}

impl<'a> HookContext<'a> {
    pub(crate) fn new(app: &'a mut AppState, path: ElementPath) -> Self {
        Self {
            app,
            path,
            pending_presentation: None,
            pending_messages: Vec::new(),
        }
    }

    pub fn path(&self) -> &ElementPath {
        &self.path
    }

    pub fn current_element(&self) -> Option<&ElementType> {
        self.app.document.get(&self.path)
    }

    pub fn get_string(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<String>> {
        self.app.get_string(path)
    }

    pub fn get_strings(&self, path: impl Into<ElementPath>) -> anyhow::Result<Vec<String>> {
        self.app.get_strings(path)
    }

    pub fn get_integer(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<i64>> {
        self.app.get_integer(path)
    }

    pub fn get_number(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<f64>> {
        self.app.get_number(path)
    }

    pub fn get_bool(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<bool>> {
        self.app.get_bool(path)
    }

    pub fn present_single_select(&mut self, spec: SingleSelectSpec) -> anyhow::Result<()> {
        self.pending_presentation = Some(PendingPresentation::SingleSelect {
            path: self.path.clone(),
            spec,
        });
        Ok(())
    }

    pub fn present_multi_select(&mut self, spec: MultiSelectSpec) -> anyhow::Result<()> {
        self.pending_presentation = Some(PendingPresentation::MultiSelect {
            path: self.path.clone(),
            spec,
        });
        Ok(())
    }

    pub fn present_input(&mut self, spec: InputPageSpec) -> anyhow::Result<()> {
        self.pending_presentation = Some(PendingPresentation::Input {
            path: self.path.clone(),
            spec,
        });
        Ok(())
    }

    pub fn show_message(&mut self, level: MessageLevel, text: impl Into<String>) {
        self.pending_messages.push(PendingMessage {
            level,
            text: text.into(),
        });
    }

    pub fn mutation(&mut self) -> HookMutation<'_> {
        HookMutation::new(self.app)
    }

    pub(crate) fn take_pending_presentation(&mut self) -> Option<PendingPresentation> {
        self.pending_presentation.take()
    }

    pub(crate) fn drain_messages(&mut self) -> Vec<PendingMessage> {
        std::mem::take(&mut self.pending_messages)
    }
}
