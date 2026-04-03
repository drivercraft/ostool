use std::{
    collections::BTreeSet,
    io::{self, Stdout},
    time::Duration,
};

use anyhow::{Context, anyhow, bail};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::data::{
    AppState, ElementPath, HookContext, HookFlow, InputBinding, InputKind, InputPageSpec,
    MessageLevel, MultiSelectBinding, MultiSelectSpec, SingleSelectBinding, SingleSelectSpec,
    hook::{PendingMessage, PendingPresentation},
    item::ItemType,
    types::ElementType,
};

mod input;
mod theme;

use input::{InputBuffer, InputBufferKind};
use theme::Theme;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MIN_TERMINAL_WIDTH: u16 = 72;
const MIN_TERMINAL_HEIGHT: u16 = 24;

pub fn run_tui(app_state: AppState) -> anyhow::Result<AppState> {
    let mut terminal = setup_terminal()?;
    let mut app = TuiApp::new(app_state);
    let run_result = app.run(&mut terminal);
    let cleanup_result = restore_terminal(&mut terminal);
    let app_state = run_result?;
    cleanup_result?;
    Ok(app_state)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Browse,
    Modal,
    ConfirmSave,
    ConfirmQuit,
    TooSmall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    Navigation,
    Detail,
}

#[derive(Debug, Clone)]
struct StatusLineState {
    level: MessageLevel,
    text: String,
}

#[derive(Debug, Clone)]
struct InlineEditorState {
    path: ElementPath,
    kind: InputBufferKind,
    buffer: InputBuffer,
    error: Option<String>,
}

#[derive(Clone)]
struct SingleSelectModal {
    path: ElementPath,
    spec: SingleSelectSpec,
    state: ListState,
}

#[derive(Clone)]
struct MultiSelectModal {
    path: ElementPath,
    spec: MultiSelectSpec,
    selected: BTreeSet<String>,
    state: ListState,
    error: Option<String>,
}

#[derive(Clone)]
struct InputModal {
    path: ElementPath,
    spec: InputPageSpec,
    buffer: InputBuffer,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct ArrayEditorModal {
    path: ElementPath,
    title: String,
    items: Vec<String>,
    state: ListState,
}

#[derive(Debug, Clone)]
struct OneOfModal {
    path: ElementPath,
    title: String,
    options: Vec<String>,
    state: ListState,
}

#[derive(Debug, Clone)]
struct HelpModal {
    title: String,
    body: String,
    scroll: u16,
}

#[derive(Debug, Clone)]
struct ValidationErrorModal {
    missing_count: usize,
    missing_fields: Vec<String>,
    scroll: u16,
}

#[derive(Debug, Clone)]
enum ConfirmAction {
    SaveAndExit,
    DiscardAndExit,
    DeleteArrayItem { path: ElementPath, index: usize },
}

#[derive(Debug, Clone)]
struct ConfirmModal {
    title: String,
    message: String,
    selected_yes: bool,
    action: ConfirmAction,
}

#[derive(Clone)]
enum ModalState {
    SingleSelect(SingleSelectModal),
    MultiSelect(MultiSelectModal),
    Input(InputModal),
    ArrayEditor(ArrayEditorModal),
    OneOfSelect(OneOfModal),
    Help(HelpModal),
    Confirm(ConfirmModal),
    ValidationError(ValidationErrorModal),
}

struct TuiState {
    mode: AppMode,
    focus: FocusTarget,
    nav_state: ListState,
    detail_scroll: u16,
    modal_stack: Vec<ModalState>,
    status: Option<StatusLineState>,
    inline_editor: Option<InlineEditorState>,
    theme: Theme,
}

impl Default for TuiState {
    fn default() -> Self {
        let mut nav_state = ListState::default();
        nav_state.select(Some(0));
        Self {
            mode: AppMode::Browse,
            focus: FocusTarget::Navigation,
            nav_state,
            detail_scroll: 0,
            modal_stack: Vec::new(),
            status: None,
            inline_editor: None,
            theme: Theme::default(),
        }
    }
}

struct TuiApp {
    state: AppState,
    ui: TuiState,
    should_exit: bool,
}

impl TuiApp {
    fn new(state: AppState) -> Self {
        Self {
            state,
            ui: TuiState::default(),
            should_exit: false,
        }
    }

    fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> anyhow::Result<AppState> {
        self.sync_navigation_state();

        while !self.should_exit {
            terminal
                .draw(|frame| self.render(frame))
                .context("failed to draw jkconfig tui")?;

            if !event::poll(EVENT_POLL_INTERVAL).context("failed to poll terminal event")? {
                continue;
            }

            let Event::Key(key) = event::read().context("failed to read terminal event")? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            self.handle_key_event(key)?;
        }

        Ok(self.state.clone())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        if self.ui.mode == AppMode::TooSmall {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                    self.open_confirm_quit();
                }
                _ => {}
            }
            return Ok(());
        }

        if self.has_modal() {
            return self.handle_modal_key_event(key);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('s')) {
            self.open_confirm_save();
            return Ok(());
        }

        if self.ui.focus == FocusTarget::Detail && self.ui.inline_editor.is_some() {
            return self.handle_inline_editor_key_event(key);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.open_confirm_quit(),
            KeyCode::Char('s') | KeyCode::Char('S') => self.open_confirm_save(),
            KeyCode::Esc => {
                if !self.state.navigate_back() {
                    self.open_confirm_quit();
                } else {
                    self.reset_detail_focus();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.move_selection(-1);
                self.reset_detail_focus();
                self.sync_navigation_state();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.move_selection(1);
                self.reset_detail_focus();
                self.sync_navigation_state();
            }
            KeyCode::PageUp => {
                self.state.move_selection(-10);
                self.reset_detail_focus();
                self.sync_navigation_state();
            }
            KeyCode::PageDown => {
                self.state.move_selection(10);
                self.reset_detail_focus();
                self.sync_navigation_state();
            }
            KeyCode::Tab => {
                if matches!(self.state.current(), Some(ElementType::OneOf(_))) {
                    self.open_oneof_modal();
                } else if self.can_focus_detail() {
                    self.ui.focus = match self.ui.focus {
                        FocusTarget::Navigation => FocusTarget::Detail,
                        FocusTarget::Detail => FocusTarget::Navigation,
                    };
                    if self.ui.focus == FocusTarget::Detail
                        && self.ui.inline_editor.is_none()
                        && self.can_focus_detail()
                    {
                        self.begin_inline_editor()?;
                    }
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if let Some(path) = self.state.selected_path()
                    && let Err(err) = self.state.clear_optional(path)
                {
                    self.set_status(MessageLevel::Warning, err.to_string());
                }
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                if let Some(path) = self.state.selected_path()
                    && let Err(err) = self.state.toggle_menu_set(path)
                {
                    self.set_status(MessageLevel::Warning, err.to_string());
                }
            }
            KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Char('?') => self.open_help_modal(),
            KeyCode::Char(' ') => {
                if let Some(path) = self.state.selected_path()
                    && let Some(ElementType::Item(item)) = self.state.current()
                    && matches!(item.item_type, ItemType::Boolean { .. })
                    && let Err(err) = self.state.toggle_bool(path)
                {
                    self.set_status(MessageLevel::Error, err.to_string());
                }
            }
            KeyCode::Enter => self.activate_selected()?,
            _ => {}
        }

        Ok(())
    }

    fn handle_inline_editor_key_event(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        match key.code {
            KeyCode::Esc => self.reset_detail_focus(),
            KeyCode::Tab => {
                self.commit_inline_editor()?;
                self.ui.focus = FocusTarget::Navigation;
            }
            KeyCode::Enter => self.commit_inline_editor()?,
            KeyCode::Left => {
                if let Some(editor) = &mut self.ui.inline_editor {
                    editor.buffer.move_left();
                    editor.error = None;
                }
            }
            KeyCode::Right => {
                if let Some(editor) = &mut self.ui.inline_editor {
                    editor.buffer.move_right();
                    editor.error = None;
                }
            }
            KeyCode::Home => {
                if let Some(editor) = &mut self.ui.inline_editor {
                    editor.buffer.move_home();
                    editor.error = None;
                }
            }
            KeyCode::End => {
                if let Some(editor) = &mut self.ui.inline_editor {
                    editor.buffer.move_end();
                    editor.error = None;
                }
            }
            KeyCode::Backspace => {
                if let Some(editor) = &mut self.ui.inline_editor {
                    editor.buffer.delete_left();
                    editor.error = None;
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(editor) = &mut self.ui.inline_editor
                    && editor.buffer.can_accept_char(editor.kind, ch)
                {
                    editor.buffer.insert_char(ch);
                    editor.error = None;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_modal_key_event(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        self.refresh_top_modal();
        let mut close_modal = false;
        let mut open_array_input: Option<(ElementPath, Option<usize>)> = None;
        let mut open_confirm: Option<ConfirmModal> = None;
        let mut confirm_action: Option<ConfirmAction> = None;

        match self.ui.modal_stack.last_mut() {
            Some(ModalState::SingleSelect(modal)) => match key.code {
                KeyCode::Esc => close_modal = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    list_previous(&mut modal.state, modal.spec.options.len())
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    list_next(&mut modal.state, modal.spec.options.len())
                }
                KeyCode::Char('c') | KeyCode::Char('C') if modal.spec.allow_clear => {
                    apply_single_select_binding(
                        &mut self.state,
                        &modal.path,
                        &modal.spec.binding,
                        None,
                    )?;
                    close_modal = true;
                }
                KeyCode::Enter => {
                    let Some(index) = modal.state.selected() else {
                        return Ok(());
                    };
                    let option = modal
                        .spec
                        .options
                        .get(index)
                        .ok_or_else(|| anyhow!("invalid selection"))?;
                    if option.disabled {
                        self.set_status(MessageLevel::Warning, "This option is disabled.");
                        return Ok(());
                    }
                    apply_single_select_binding(
                        &mut self.state,
                        &modal.path,
                        &modal.spec.binding,
                        Some(option.value.clone()),
                    )?;
                    close_modal = true;
                }
                _ => {}
            },
            Some(ModalState::MultiSelect(modal)) => match key.code {
                KeyCode::Esc => close_modal = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    list_previous(&mut modal.state, modal.spec.options.len())
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    list_next(&mut modal.state, modal.spec.options.len())
                }
                KeyCode::Char(' ') => {
                    let Some(index) = modal.state.selected() else {
                        return Ok(());
                    };
                    let Some(option) = modal.spec.options.get(index) else {
                        return Ok(());
                    };
                    if option.disabled {
                        modal.error = Some("This option is disabled.".into());
                        return Ok(());
                    }
                    if modal.selected.contains(&option.value) {
                        modal.selected.remove(&option.value);
                    } else {
                        modal.selected.insert(option.value.clone());
                    }
                    modal.error = None;
                }
                KeyCode::Enter => {
                    let values = ordered_multi_selection(&modal.spec, &modal.selected);
                    if let Some(min_selected) = modal.spec.min_selected
                        && values.len() < min_selected
                    {
                        modal.error =
                            Some(format!("Please select at least {min_selected} item(s)."));
                        return Ok(());
                    }
                    if let Some(max_selected) = modal.spec.max_selected
                        && values.len() > max_selected
                    {
                        modal.error =
                            Some(format!("Please select at most {max_selected} item(s)."));
                        return Ok(());
                    }
                    apply_multi_select_binding(
                        &mut self.state,
                        &modal.path,
                        &modal.spec.binding,
                        values,
                    )?;
                    close_modal = true;
                }
                _ => {}
            },
            Some(ModalState::Input(modal)) => match key.code {
                KeyCode::Esc => close_modal = true,
                KeyCode::Left => {
                    modal.buffer.move_left();
                    modal.error = None;
                }
                KeyCode::Right => {
                    modal.buffer.move_right();
                    modal.error = None;
                }
                KeyCode::Home => {
                    modal.buffer.move_home();
                    modal.error = None;
                }
                KeyCode::End => {
                    modal.buffer.move_end();
                    modal.error = None;
                }
                KeyCode::Backspace => {
                    modal.buffer.delete_left();
                    modal.error = None;
                }
                KeyCode::Enter => {
                    if let Err(err) = apply_input_binding(
                        &mut self.state,
                        &modal.path,
                        &modal.spec,
                        modal.buffer.value().to_string(),
                    ) {
                        modal.error = Some(err.to_string());
                    } else {
                        close_modal = true;
                    }
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if modal
                        .buffer
                        .can_accept_char(kind_for_input(modal.spec.kind), ch)
                    {
                        modal.buffer.insert_char(ch);
                        modal.error = None;
                    }
                }
                _ => {}
            },
            Some(ModalState::ArrayEditor(modal)) => match key.code {
                KeyCode::Esc => close_modal = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    list_previous(&mut modal.state, modal.items.len().max(1))
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    list_next(&mut modal.state, modal.items.len().max(1))
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    open_array_input = Some((modal.path.clone(), None));
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    if let Some(index) = modal.state.selected()
                        && index < modal.items.len()
                    {
                        open_confirm = Some(ConfirmModal {
                            title: "Delete Array Item".into(),
                            message: format!("Delete [{}] {}?", index, modal.items[index]),
                            selected_yes: false,
                            action: ConfirmAction::DeleteArrayItem {
                                path: modal.path.clone(),
                                index,
                            },
                        });
                    }
                }
                KeyCode::Enter => {
                    if let Some(index) = modal.state.selected()
                        && index < modal.items.len()
                    {
                        open_array_input = Some((modal.path.clone(), Some(index)));
                    }
                }
                _ => {}
            },
            Some(ModalState::OneOfSelect(modal)) => match key.code {
                KeyCode::Esc => close_modal = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    list_previous(&mut modal.state, modal.options.len())
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    list_next(&mut modal.state, modal.options.len())
                }
                KeyCode::Enter => {
                    let Some(index) = modal.state.selected() else {
                        return Ok(());
                    };
                    self.state.set_oneof_index(modal.path.clone(), index)?;
                    close_modal = true;
                }
                _ => {}
            },
            Some(ModalState::Help(modal)) => match key.code {
                KeyCode::Esc => close_modal = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    modal.scroll = modal.scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    modal.scroll = modal.scroll.saturating_add(1);
                }
                _ => {}
            },
            Some(ModalState::Confirm(modal)) => match key.code {
                KeyCode::Esc => close_modal = true,
                KeyCode::Left | KeyCode::Char('h') => modal.selected_yes = true,
                KeyCode::Right | KeyCode::Char('l') => modal.selected_yes = false,
                KeyCode::Tab => modal.selected_yes = !modal.selected_yes,
                KeyCode::Enter => {
                    if modal.selected_yes {
                        confirm_action = Some(modal.action.clone());
                    } else {
                        close_modal = true;
                    }
                }
                _ => {}
            },
            Some(ModalState::ValidationError(_)) => match key.code {
                KeyCode::Esc | KeyCode::Enter => close_modal = true,
                _ => {}
            },
            None => {}
        }

        if let Some((path, index)) = open_array_input {
            self.open_array_input_modal(path, index)?;
        }
        if let Some(confirm) = open_confirm {
            self.open_confirm(confirm);
        }
        if let Some(action) = confirm_action {
            self.apply_confirm_action(action)?;
        } else if close_modal {
            self.close_top_modal();
        }

        Ok(())
    }

    fn activate_selected(&mut self) -> anyhow::Result<()> {
        let Some(path) = self.state.selected_path() else {
            return Ok(());
        };

        if let Some(hook) = self.state.find_selected_hook() {
            let (flow, presentation, messages) = {
                let mut context = HookContext::new(&mut self.state, path.clone());
                let flow = (hook.callback)(&mut context, &path)?;
                let presentation = context.take_pending_presentation();
                let messages = context.drain_messages();
                (flow, presentation, messages)
            };
            self.consume_hook_side_effects(presentation, messages);
            if flow == HookFlow::Consumed {
                self.reset_detail_focus();
                return Ok(());
            }
        }

        let Some(element) = self.state.document.get(&path).cloned() else {
            return Ok(());
        };

        match element {
            ElementType::Menu(menu) => {
                if !menu.is_required && !menu.is_set {
                    let _ = self.state.toggle_menu_set(path.clone());
                }
                self.state.enter_menu(path);
                self.reset_detail_focus();
                self.sync_navigation_state();
            }
            ElementType::OneOf(one_of) => {
                if matches!(one_of.selected(), Some(ElementType::Menu(_))) {
                    self.state.enter_menu(path);
                    self.reset_detail_focus();
                    self.sync_navigation_state();
                } else {
                    self.open_oneof_modal();
                }
            }
            ElementType::Item(item) => match &item.item_type {
                ItemType::Boolean { .. } => {
                    self.state.toggle_bool(path)?;
                }
                ItemType::String { .. } | ItemType::Integer { .. } | ItemType::Number { .. } => {
                    if self.can_focus_detail() {
                        self.ui.focus = FocusTarget::Detail;
                        self.begin_inline_editor()?;
                    } else {
                        self.open_default_input_modal(path)?;
                    }
                }
                ItemType::Enum(enum_item) => {
                    let options = enum_item
                        .variants
                        .iter()
                        .cloned()
                        .map(|variant| crate::data::HookOption::new(variant.clone(), variant))
                        .collect();
                    self.open_modal(ModalState::SingleSelect(SingleSelectModal::new(
                        path,
                        SingleSelectSpec {
                            title: format!("Select {}", item.base.title),
                            help: item.base.help.clone(),
                            options,
                            initial: enum_item.value_str().map(ToString::to_string),
                            allow_clear: !item.base.is_required,
                            binding: SingleSelectBinding::SetEnumVariant {
                                path: ElementPath::parse(&item.base.key()),
                            },
                        },
                    )));
                }
                ItemType::Array(_) => self.open_array_modal(path)?,
            },
        }

        Ok(())
    }

    fn begin_inline_editor(&mut self) -> anyhow::Result<()> {
        let Some(path) = self.state.selected_path() else {
            return Ok(());
        };
        let Some(element) = self.state.document.get(&path) else {
            return Ok(());
        };

        let (kind, initial) = match element {
            ElementType::Item(item) => match &item.item_type {
                ItemType::String { value, default } => (
                    InputBufferKind::Text,
                    value
                        .clone()
                        .or_else(|| default.clone())
                        .unwrap_or_default(),
                ),
                ItemType::Integer { value, default } => (
                    InputBufferKind::Integer,
                    value
                        .or(*default)
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                ItemType::Number { value, default } => (
                    InputBufferKind::Number,
                    value
                        .or(*default)
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                _ => bail!("current item cannot be edited inline"),
            },
            _ => bail!("current element cannot be edited inline"),
        };

        self.ui.inline_editor = Some(InlineEditorState {
            path,
            kind,
            buffer: InputBuffer::new(initial),
            error: None,
        });
        Ok(())
    }

    fn commit_inline_editor(&mut self) -> anyhow::Result<()> {
        let Some(editor) = &mut self.ui.inline_editor else {
            return Ok(());
        };

        let result = match editor.kind {
            InputBufferKind::Text => self
                .state
                .set_string(editor.path.clone(), editor.buffer.value().to_string()),
            InputBufferKind::Integer => self
                .state
                .set_integer(editor.path.clone(), editor.buffer.parse_i64()?),
            InputBufferKind::Number => self
                .state
                .set_number(editor.path.clone(), editor.buffer.parse_f64()?),
        };

        match result {
            Ok(()) => {
                self.ui.inline_editor = None;
                Ok(())
            }
            Err(err) => {
                editor.error = Some(err.to_string());
                Ok(())
            }
        }
    }

    fn consume_hook_side_effects(
        &mut self,
        presentation: Option<PendingPresentation>,
        messages: Vec<PendingMessage>,
    ) {
        if let Some(presentation) = presentation {
            match presentation {
                PendingPresentation::SingleSelect { path, spec } => {
                    self.open_modal(ModalState::SingleSelect(SingleSelectModal::new(path, spec)));
                }
                PendingPresentation::MultiSelect { path, spec } => {
                    self.open_modal(ModalState::MultiSelect(MultiSelectModal::new(path, spec)));
                }
                PendingPresentation::Input { path, spec } => {
                    self.open_modal(ModalState::Input(InputModal::new(path, spec)));
                }
            }
        }

        for PendingMessage { level, text } in messages {
            self.set_status(level, text);
        }
    }

    fn open_confirm_save(&mut self) {
        self.open_confirm(ConfirmModal {
            title: "Save Changes".into(),
            message: "Save current changes and exit?".into(),
            selected_yes: true,
            action: ConfirmAction::SaveAndExit,
        });
        self.ui.mode = AppMode::ConfirmSave;
    }

    fn open_confirm_quit(&mut self) {
        self.open_confirm(ConfirmModal {
            title: "Quit Without Saving".into(),
            message: "Discard current edits and quit?".into(),
            selected_yes: false,
            action: ConfirmAction::DiscardAndExit,
        });
        self.ui.mode = AppMode::ConfirmQuit;
    }

    fn open_help_modal(&mut self) {
        self.open_modal(ModalState::Help(HelpModal {
            title: "Item Details".into(),
            body: self.state.selected_detail_text(),
            scroll: 0,
        }));
    }

    fn open_validation_modal(&mut self, missing: &[String]) {
        self.set_status(
            MessageLevel::Error,
            format!("Cannot save: {} required field(s) missing.", missing.len()),
        );
        self.open_modal(ModalState::ValidationError(ValidationErrorModal {
            missing_count: missing.len(),
            missing_fields: missing.to_vec(),
            scroll: 0,
        }));
    }

    fn open_oneof_modal(&mut self) {
        let Some(path) = self.state.selected_path() else {
            return;
        };
        let Some(ElementType::OneOf(one_of)) = self.state.document.get(&path) else {
            return;
        };
        let options = one_of
            .variants
            .iter()
            .enumerate()
            .map(|(idx, _)| one_of.variant_display(idx))
            .collect::<Vec<_>>();

        let mut state = ListState::default();
        state.select(one_of.selected_index.or(Some(0)));
        self.open_modal(ModalState::OneOfSelect(OneOfModal {
            path,
            title: one_of.title.clone(),
            options,
            state,
        }));
    }

    fn open_array_modal(&mut self, path: ElementPath) -> anyhow::Result<()> {
        let items = self.state.get_strings(path.clone())?;
        let title = self
            .state
            .document
            .get(&path)
            .map(|element| element.title.clone())
            .unwrap_or_else(|| "Array Editor".into());
        let mut state = ListState::default();
        state.select(Some(0));
        self.open_modal(ModalState::ArrayEditor(ArrayEditorModal {
            path,
            title,
            items,
            state,
        }));
        Ok(())
    }

    fn open_default_input_modal(&mut self, path: ElementPath) -> anyhow::Result<()> {
        let Some(element) = self.state.document.get(&path) else {
            return Ok(());
        };
        let (kind, initial, allow_empty, binding, label) = match element {
            ElementType::Item(item) => match &item.item_type {
                ItemType::String { value, default } => (
                    InputKind::Text,
                    value.clone().or_else(|| default.clone()),
                    !item.base.is_required,
                    if item.base.is_required {
                        InputBinding::SetString { path: path.clone() }
                    } else {
                        InputBinding::SetOptionalString { path: path.clone() }
                    },
                    item.base.title.clone(),
                ),
                ItemType::Integer { value, default } => (
                    InputKind::Integer,
                    value.or(*default).map(|value| value.to_string()),
                    !item.base.is_required,
                    if item.base.is_required {
                        InputBinding::SetInteger { path: path.clone() }
                    } else {
                        InputBinding::SetOptionalInteger { path: path.clone() }
                    },
                    item.base.title.clone(),
                ),
                ItemType::Number { value, default } => (
                    InputKind::Number,
                    value.or(*default).map(|value| value.to_string()),
                    !item.base.is_required,
                    if item.base.is_required {
                        InputBinding::SetNumber { path: path.clone() }
                    } else {
                        InputBinding::SetOptionalNumber { path: path.clone() }
                    },
                    item.base.title.clone(),
                ),
                _ => bail!("element cannot be edited in an input modal"),
            },
            _ => bail!("element cannot be edited in an input modal"),
        };

        self.open_modal(ModalState::Input(InputModal::new(
            path.clone(),
            InputPageSpec {
                title: format!("Edit {label}"),
                label,
                help: element.help.clone(),
                initial,
                placeholder: None,
                kind,
                allow_empty,
                min_inner_width: if matches!(kind, InputKind::Path | InputKind::Command) {
                    40
                } else {
                    32
                },
                binding,
            },
        )));

        Ok(())
    }

    fn open_array_input_modal(
        &mut self,
        path: ElementPath,
        index: Option<usize>,
    ) -> anyhow::Result<()> {
        let values = self.state.get_strings(path.clone())?;
        let initial = index.and_then(|index| values.get(index).cloned());
        let title = if let Some(index) = index {
            format!("Edit Array Item [{index}]")
        } else {
            "Add Array Item".to_string()
        };
        let target_path = path.clone();
        let binding = InputBinding::Custom(std::sync::Arc::new(move |tx, value| {
            let mut next = values.clone();
            if let Some(index) = index {
                if index >= next.len() {
                    bail!("array index out of bounds");
                }
                next[index] = value;
            } else {
                next.push(value);
            }
            tx.set_string_array(target_path.clone(), next)
        }));

        self.open_modal(ModalState::Input(InputModal::new(
            path.clone(),
            InputPageSpec {
                title,
                label: "Value".into(),
                help: None,
                initial,
                placeholder: None,
                kind: InputKind::Text,
                allow_empty: false,
                min_inner_width: 40,
                binding,
            },
        )));
        Ok(())
    }

    fn apply_confirm_action(&mut self, action: ConfirmAction) -> anyhow::Result<()> {
        match action {
            ConfirmAction::SaveAndExit => {
                let missing = self.state.missing_required_paths();
                if !missing.is_empty() {
                    self.close_top_modal();
                    self.open_validation_modal(&missing);
                    return Ok(());
                }
                self.should_exit = true;
            }
            ConfirmAction::DiscardAndExit => {
                self.state.discard_changes();
                self.should_exit = true;
            }
            ConfirmAction::DeleteArrayItem { path, index } => {
                let mut items = self.state.get_strings(path.clone())?;
                if index >= items.len() {
                    bail!("array index out of bounds");
                }
                items.remove(index);
                self.state.set_string_array(path, items)?;
                self.close_top_modal();
            }
        }
        self.close_top_modal();
        Ok(())
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let theme = self.ui.theme;

        frame.render_widget(Block::default().style(theme.text()), area);

        if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
            self.ui.mode = AppMode::TooSmall;
            self.render_too_small(frame);
            return;
        }

        if self.has_modal() {
            self.ui.mode = match self.ui.mode {
                AppMode::ConfirmSave => AppMode::ConfirmSave,
                AppMode::ConfirmQuit => AppMode::ConfirmQuit,
                _ => AppMode::Modal,
            };
        } else {
            self.ui.mode = AppMode::Browse;
        }

        // Vertical layout: Header | Navigation | Detail | Editor | Footer
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header (1 line + border)
                Constraint::Max(14),   // Navigation (scrollable list)
                Constraint::Min(8),    // Detail (full info display)
                Constraint::Length(5), // Editor / Action bar (input + hint + borders)
                Constraint::Length(4), // Footer (shortcuts, 2 lines + border)
            ])
            .split(area);

        self.render_header(frame, layout[0]);
        self.render_navigation(frame, layout[1]);
        self.render_detail(frame, layout[2]);
        self.render_editor_or_action(frame, layout[3]);
        self.render_footer(frame, layout[4]);

        if self.has_modal() {
            self.render_modal(frame);
        }
    }

    fn render_too_small(&mut self, frame: &mut Frame) {
        let area = centered_rect(frame.area(), 70, 7);
        frame.render_widget(Clear, area);
        let paragraph = Paragraph::new(Text::from(vec![
            Line::from("Terminal is too small for jkconfig."),
            Line::from("Minimum size: 72 columns x 24 rows."),
            Line::from("Resize the terminal or press q / Esc to quit."),
        ]))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title(" Screen Too Small ")
                .borders(Borders::ALL)
                .border_style(self.ui.theme.active_border()),
        );
        frame.render_widget(paragraph, area);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let title = self.state.document.title();
        let dirty = if self.state.needs_save { "*" } else { "" };
        let text = Line::from(vec![
            Span::styled(" JKConfig ", self.ui.theme.accent()),
            Span::styled(
                title.to_string(),
                self.ui.theme.text().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(self.state.current_path_string(), self.ui.theme.muted()),
            Span::raw("  "),
            Span::styled(
                if dirty.is_empty() {
                    "".into()
                } else {
                    dirty.to_string()
                },
                if dirty.is_empty() {
                    self.ui.theme.muted()
                } else {
                    Style::default().fg(self.ui.theme.error)
                },
            ),
        ]);
        frame.render_widget(
            Paragraph::new(text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.ui.theme.passive_border()),
            ),
            area,
        );
    }

    fn render_navigation(&mut self, frame: &mut Frame, area: Rect) {
        let Some(menu) = self.state.current_menu() else {
            return;
        };

        // Compute max title width for alignment
        let max_title_len = menu
            .children
            .iter()
            .map(|e| e.title.len())
            .max()
            .unwrap_or(0);

        let items = menu
            .children
            .iter()
            .map(|element| {
                let summary = AppState::element_summary(element);
                let is_required_unset = element.is_required && element.is_none();
                let mut spans: Vec<Span> = vec![];

                // Fixed-width marker column: always 2 chars to keep alignment
                if is_required_unset {
                    spans.push(Span::styled("* ", self.ui.theme.required()));
                } else {
                    spans.push(Span::raw("  "));
                }

                // Tag column: fixed width [XXX]
                spans.push(Span::styled(
                    format!("[{:>3}] ", AppState::element_tag(element)),
                    if is_required_unset {
                        self.ui.theme.required_dim()
                    } else {
                        self.ui.theme.accent()
                    },
                ));

                // Title column: padded to align summary
                let padded_title = format!("{:<width$}", element.title, width = max_title_len);
                spans.push(Span::styled(
                    padded_title,
                    if is_required_unset {
                        self.ui.theme.required()
                    } else {
                        self.ui.theme.text()
                    },
                ));

                spans.push(Span::raw("  "));
                spans.push(Span::styled(summary, self.ui.theme.muted()));
                ListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>();

        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .title(format!(" {} ", menu.title))
                        .borders(Borders::ALL)
                        .border_style(if self.ui.focus == FocusTarget::Navigation {
                            self.ui.theme.active_border()
                        } else {
                            self.ui.theme.passive_border()
                        }),
                )
                .highlight_style(self.ui.theme.selected_row()),
            area,
            &mut self.ui.nav_state,
        );
    }

    fn render_detail(&mut self, frame: &mut Frame, area: Rect) {
        let detail_text = self.state.selected_detail_text();
        frame.render_widget(
            Paragraph::new(detail_text)
                .wrap(Wrap { trim: false })
                .scroll((self.ui.detail_scroll, 0))
                .block(
                    Block::default()
                        .title(" Details ")
                        .borders(Borders::ALL)
                        .border_style(self.ui.theme.passive_border()),
                ),
            area,
        );
    }

    fn render_editor_or_action(&mut self, frame: &mut Frame, area: Rect) {
        let Some(element) = self.state.current() else {
            return;
        };

        let show_inline = self.can_focus_detail();
        match element {
            ElementType::Item(item)
                if matches!(
                    item.item_type,
                    ItemType::String { .. } | ItemType::Integer { .. } | ItemType::Number { .. }
                ) && show_inline =>
            {
                let editor =
                    self.ui.inline_editor.as_ref().filter(|editor| {
                        editor.path == self.state.selected_path().unwrap_or_default()
                    });
                let (text, cursor_offset, error) = if let Some(editor) = editor {
                    let (visible, cursor_offset) = editor
                        .buffer
                        .visible_text_and_cursor(area.width.saturating_sub(2) as usize);
                    (visible, Some(cursor_offset), editor.error.clone())
                } else {
                    let initial = AppState::element_summary(element);
                    let cursor_offset = initial.len() as u16;
                    (initial, Some(cursor_offset), None)
                };

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(2)])
                    .split(area);
                let input_area = chunks[0];
                let hint_area = chunks[1];

                frame.render_widget(
                    Paragraph::new(text).block(
                        Block::default()
                            .title(format!(" Edit {} ", item.base.title))
                            .borders(Borders::ALL)
                            .border_style(if self.ui.focus == FocusTarget::Detail {
                                self.ui.theme.active_border()
                            } else {
                                self.ui.theme.passive_border()
                            }),
                    ),
                    input_area,
                );
                let helper = error.unwrap_or_else(|| {
                    if self.ui.focus == FocusTarget::Detail {
                        "Enter commit  Esc cancel  Tab back".into()
                    } else {
                        "Tab to focus editor  Enter to edit".into()
                    }
                });
                frame.render_widget(
                    Paragraph::new(helper).style(self.ui.theme.muted()),
                    hint_area,
                );

                if self.ui.focus == FocusTarget::Detail
                    && let Some(editor) = &self.ui.inline_editor
                    && editor.path == self.state.selected_path().unwrap_or_default()
                    && let Some(cursor_offset) = cursor_offset
                {
                    frame.set_cursor_position(Position::new(
                        input_area.x + 1 + cursor_offset,
                        input_area.y + 1,
                    ));
                }
            }
            _ => {
                let text = match element {
                    ElementType::Item(item) => match &item.item_type {
                        ItemType::Boolean { .. } => "Space toggle",
                        ItemType::Enum(_) => "Enter choose variant",
                        ItemType::Array(_) => "Enter open array editor",
                        ItemType::String { .. }
                        | ItemType::Integer { .. }
                        | ItemType::Number { .. } => "Enter edit in dialog",
                    },
                    ElementType::Menu(_) => "Enter navigate  M toggle optional",
                    ElementType::OneOf(_) => "Enter select  Tab switch variant",
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(text, self.ui.theme.muted()),
                        Span::raw("  "),
                        Span::styled("? help", self.ui.theme.muted()),
                    ]))
                    .block(
                        Block::default()
                            .title(" Actions ")
                            .borders(Borders::ALL)
                            .border_style(self.ui.theme.passive_border()),
                    ),
                    area,
                );
            }
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let key = |k: &str| Span::styled(format!("[{k}]"), self.ui.theme.accent());
        let sp = || Span::raw("  ");
        let txt = |s: &str| Span::styled(s.to_string(), self.ui.theme.muted());

        let (content, style) = if let Some(status) = &self.ui.status {
            (
                Text::from(Line::from(status.text.clone())),
                match status.level {
                    MessageLevel::Success => Style::default().fg(self.ui.theme.success),
                    MessageLevel::Warning => Style::default().fg(self.ui.theme.warning),
                    MessageLevel::Error => Style::default().fg(self.ui.theme.error),
                    MessageLevel::Info => self.ui.theme.text(),
                },
            )
        } else {
            (
                Text::from(vec![
                    Line::from(vec![
                        key("j/k"),
                        txt("move"),
                        sp(),
                        key("Enter"),
                        txt("open/edit"),
                        sp(),
                        key("Tab"),
                        txt("focus"),
                        sp(),
                        key("Space"),
                        txt("toggle bool"),
                    ]),
                    Line::from(vec![
                        key("c"),
                        txt("clear"),
                        sp(),
                        key("m"),
                        txt("toggle opt"),
                        sp(),
                        key("s"),
                        txt("save"),
                        sp(),
                        key("q"),
                        txt("quit"),
                        sp(),
                        key("?"),
                        txt("help"),
                    ]),
                ]),
                Style::default(),
            )
        };

        frame.render_widget(
            Paragraph::new(content).style(style).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.ui.theme.passive_border()),
            ),
            area,
        );
    }

    fn render_modal(&mut self, frame: &mut Frame) {
        let Some(modal) = self.ui.modal_stack.last() else {
            return;
        };

        match modal {
            ModalState::SingleSelect(modal) => {
                let area = centered_rect(frame.area(), 76, 18);
                frame.render_widget(Clear, area);
                let modal_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Min(8),
                        Constraint::Length(4),
                    ])
                    .margin(1)
                    .split(area);
                let help_area = modal_layout[0];
                let list_area = modal_layout[1];
                let detail_area = modal_layout[2];
                frame.render_widget(
                    Block::default()
                        .title(format!(" {} ", modal.spec.title))
                        .borders(Borders::ALL)
                        .border_style(self.ui.theme.active_border()),
                    area,
                );
                let help = modal
                    .spec
                    .help
                    .clone()
                    .unwrap_or_else(|| "Enter apply  Esc close".into());
                frame.render_widget(Paragraph::new(help).style(self.ui.theme.muted()), help_area);
                let items = modal
                    .spec
                    .options
                    .iter()
                    .map(|option| {
                        let prefix = if Some(option.value.clone()) == modal.spec.initial {
                            "(*) "
                        } else {
                            "( ) "
                        };
                        let label_style = if option.disabled {
                            self.ui.theme.muted()
                        } else {
                            self.ui.theme.text()
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(prefix, self.ui.theme.accent()),
                            Span::styled(option.label.clone(), label_style),
                        ]))
                    })
                    .collect::<Vec<_>>();
                let mut state = modal.state;
                frame.render_stateful_widget(
                    List::new(items).highlight_style(self.ui.theme.selected_row()),
                    list_area,
                    &mut state,
                );
                let selected_detail = modal
                    .state
                    .selected()
                    .and_then(|index| modal.spec.options.get(index))
                    .and_then(|option| option.detail.clone())
                    .unwrap_or_else(|| "Use Up/Down to select an option.".into());
                frame.render_widget(
                    Paragraph::new(selected_detail)
                        .wrap(Wrap { trim: false })
                        .block(Block::default().borders(Borders::TOP)),
                    detail_area,
                );
            }
            ModalState::MultiSelect(modal) => {
                let area = centered_rect(frame.area(), 80, 20);
                frame.render_widget(Clear, area);
                let modal_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Min(10),
                        Constraint::Length(4),
                    ])
                    .margin(1)
                    .split(area);
                let help_area = modal_layout[0];
                let list_area = modal_layout[1];
                let footer_area = modal_layout[2];
                frame.render_widget(
                    Block::default()
                        .title(format!(" {} ", modal.spec.title))
                        .borders(Borders::ALL)
                        .border_style(self.ui.theme.active_border()),
                    area,
                );
                let help = modal
                    .spec
                    .help
                    .clone()
                    .unwrap_or_else(|| "Space toggle  Enter apply  Esc close".into());
                frame.render_widget(Paragraph::new(help).style(self.ui.theme.muted()), help_area);
                let items = modal
                    .spec
                    .options
                    .iter()
                    .map(|option| {
                        let marker = if modal.selected.contains(&option.value) {
                            "[x] "
                        } else {
                            "[ ] "
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(marker, self.ui.theme.accent()),
                            Span::styled(
                                option.label.clone(),
                                if option.disabled {
                                    self.ui.theme.muted()
                                } else {
                                    self.ui.theme.text()
                                },
                            ),
                        ]))
                    })
                    .collect::<Vec<_>>();
                let mut state = modal.state;
                frame.render_stateful_widget(
                    List::new(items).highlight_style(self.ui.theme.selected_row()),
                    list_area,
                    &mut state,
                );
                let footer = modal
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("Selected: {}", modal.selected.len()));
                frame.render_widget(
                    Paragraph::new(footer).style(self.ui.theme.muted()),
                    footer_area,
                );
            }
            ModalState::Input(modal) => {
                let width = modal
                    .spec
                    .min_inner_width
                    .saturating_add(2)
                    .min(frame.area().width.saturating_sub(4).max(10));
                let area = centered_rect(frame.area(), width, 11);
                frame.render_widget(Clear, area);
                let modal_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Length(3),
                        Constraint::Length(3),
                    ])
                    .margin(1)
                    .split(area);
                let help_area = modal_layout[0];
                let input_area = modal_layout[1];
                let error_area = modal_layout[2];
                frame.render_widget(
                    Block::default()
                        .title(format!(" {} ", modal.spec.title))
                        .borders(Borders::ALL)
                        .border_style(self.ui.theme.active_border()),
                    area,
                );
                let help = modal
                    .spec
                    .help
                    .clone()
                    .unwrap_or_else(|| "Enter apply  Esc close".into());
                frame.render_widget(Paragraph::new(help).style(self.ui.theme.muted()), help_area);
                let (visible, cursor_offset) = modal
                    .buffer
                    .visible_text_and_cursor(input_area.width.saturating_sub(2) as usize);
                frame.render_widget(
                    Paragraph::new(visible).block(
                        Block::default()
                            .title(format!(" {} ", modal.spec.label))
                            .borders(Borders::ALL)
                            .border_style(self.ui.theme.active_border()),
                    ),
                    input_area,
                );
                let error = modal
                    .error
                    .clone()
                    .unwrap_or_else(|| "Type to edit the value.".into());
                frame.render_widget(
                    Paragraph::new(error).style(self.ui.theme.muted()),
                    error_area,
                );
                frame.set_cursor_position(Position::new(
                    input_area.x + 1 + cursor_offset,
                    input_area.y + 1,
                ));
            }
            ModalState::ArrayEditor(modal) => {
                let area = centered_rect(frame.area(), 80, 20);
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Block::default()
                        .title(format!(" Array: {} ", modal.title))
                        .borders(Borders::ALL)
                        .border_style(self.ui.theme.active_border()),
                    area,
                );
                let modal_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(10), Constraint::Length(3)])
                    .margin(1)
                    .split(area);
                let list_area = modal_layout[0];
                let help_area = modal_layout[1];
                let items = if modal.items.is_empty() {
                    vec![ListItem::new(Line::from(
                        "No items yet. Press A to add one.",
                    ))]
                } else {
                    modal
                        .items
                        .iter()
                        .enumerate()
                        .map(|(idx, value)| ListItem::new(Line::from(format!("[{idx}] {value}"))))
                        .collect()
                };
                let mut state = modal.state;
                frame.render_stateful_widget(
                    List::new(items).highlight_style(self.ui.theme.selected_row()),
                    list_area,
                    &mut state,
                );
                frame.render_widget(
                    Paragraph::new("A add  Enter edit  D delete  Esc close")
                        .style(self.ui.theme.muted()),
                    help_area,
                );
            }
            ModalState::OneOfSelect(modal) => {
                let area = centered_rect(frame.area(), 70, 16);
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Block::default()
                        .title(format!(" OneOf: {} ", modal.title))
                        .borders(Borders::ALL)
                        .border_style(self.ui.theme.active_border()),
                    area,
                );
                let items = modal
                    .options
                    .iter()
                    .map(|option| ListItem::new(Line::from(option.clone())))
                    .collect::<Vec<_>>();
                let mut state = modal.state;
                frame.render_stateful_widget(
                    List::new(items)
                        .highlight_style(self.ui.theme.selected_row())
                        .block(Block::default().borders(Borders::NONE)),
                    area.inner(Margin {
                        vertical: 1,
                        horizontal: 1,
                    }),
                    &mut state,
                );
            }
            ModalState::Help(modal) => {
                let area = centered_rect(frame.area(), 80, 20);
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(modal.body.clone())
                        .wrap(Wrap { trim: false })
                        .scroll((modal.scroll, 0))
                        .block(
                            Block::default()
                                .title(format!(" {} ", modal.title))
                                .borders(Borders::ALL)
                                .border_style(self.ui.theme.active_border()),
                        ),
                    area,
                );
            }
            ModalState::ValidationError(modal) => {
                let width = 70.min(frame.area().width.saturating_sub(4));
                let height = (8 + modal.missing_fields.len() as u16).min(22);
                let area = centered_rect(frame.area(), width, height);
                frame.render_widget(Clear, area);

                let inner = area.inner(Margin {
                    vertical: 1,
                    horizontal: 1,
                });

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(4),
                        Constraint::Length(3),
                    ])
                    .split(inner);

                // Border with error style
                frame.render_widget(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(self.ui.theme.error)),
                    area,
                );

                // Title bar with icon
                let title_text = Line::from(vec![
                    Span::styled(
                        " X ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(self.ui.theme.error)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!(
                            " Cannot Save — {} Required Field(s) Missing ",
                            modal.missing_count
                        ),
                        Style::default()
                            .fg(self.ui.theme.error)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]);
                frame.render_widget(Paragraph::new(title_text), chunks[0]);

                // Field list
                let field_lines: Vec<Line> = modal
                    .missing_fields
                    .iter()
                    .map(|field| {
                        Line::from(vec![
                            Span::styled("  ", self.ui.theme.text()),
                            Span::styled("\u{2717} ", Style::default().fg(self.ui.theme.error)),
                            Span::styled(field.clone(), self.ui.theme.text()),
                        ])
                    })
                    .collect();
                frame.render_widget(
                    Paragraph::new(field_lines)
                        .scroll((modal.scroll, 0))
                        .block(Block::default().borders(Borders::NONE)),
                    chunks[1],
                );

                // Footer hint
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Press ", self.ui.theme.muted()),
                        Span::styled("Enter", self.ui.theme.text().add_modifier(Modifier::BOLD)),
                        Span::styled(" or ", self.ui.theme.muted()),
                        Span::styled("Esc", self.ui.theme.text().add_modifier(Modifier::BOLD)),
                        Span::styled(
                            " to close and fill in the missing fields.",
                            self.ui.theme.muted(),
                        ),
                    ])),
                    chunks[2],
                );
            }
            ModalState::Confirm(modal) => {
                let area = centered_rect(frame.area(), 64, 8);
                frame.render_widget(Clear, area);
                let modal_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(3), Constraint::Length(3)])
                    .margin(1)
                    .split(area);
                let body_area = modal_layout[0];
                let buttons_area = modal_layout[1];
                frame.render_widget(
                    Block::default()
                        .title(format!(" {} ", modal.title))
                        .borders(Borders::ALL)
                        .border_style(self.ui.theme.active_border()),
                    area,
                );
                frame.render_widget(
                    Paragraph::new(modal.message.clone()).alignment(Alignment::Center),
                    body_area,
                );
                let yes_style = if modal.selected_yes {
                    self.ui.theme.selected_row()
                } else {
                    self.ui.theme.text()
                };
                let no_style = if modal.selected_yes {
                    self.ui.theme.text()
                } else {
                    self.ui.theme.selected_row()
                };
                let buttons = Line::from(vec![
                    Span::styled("  Yes  ", yes_style),
                    Span::raw("    "),
                    Span::styled("  No  ", no_style),
                ]);
                frame.render_widget(
                    Paragraph::new(buttons).alignment(Alignment::Center),
                    buttons_area,
                );
            }
        }
    }

    fn sync_navigation_state(&mut self) {
        self.ui.nav_state.select(Some(self.state.selected_index()));
    }

    fn reset_detail_focus(&mut self) {
        self.ui.focus = FocusTarget::Navigation;
        self.ui.inline_editor = None;
    }

    fn can_focus_detail(&self) -> bool {
        matches!(
            self.state.current(),
            Some(ElementType::Item(item))
                if matches!(
                    item.item_type,
                    ItemType::String { .. } | ItemType::Integer { .. } | ItemType::Number { .. }
                )
        )
    }

    fn has_modal(&self) -> bool {
        !self.ui.modal_stack.is_empty()
    }

    fn open_modal(&mut self, modal: ModalState) {
        self.ui.modal_stack.push(modal);
        self.ui.mode = AppMode::Modal;
    }

    fn open_confirm(&mut self, modal: ConfirmModal) {
        self.ui.modal_stack.push(ModalState::Confirm(modal));
    }

    fn close_top_modal(&mut self) {
        self.ui.modal_stack.pop();
        if !self.has_modal() {
            self.ui.mode = AppMode::Browse;
        }
    }

    fn refresh_top_modal(&mut self) {
        if let Some(ModalState::ArrayEditor(modal)) = self.ui.modal_stack.last_mut() {
            modal.items = self
                .state
                .get_strings(modal.path.clone())
                .unwrap_or_default();
            if modal.items.is_empty() {
                modal.state.select(Some(0));
            } else if modal.state.selected().unwrap_or(0) >= modal.items.len() {
                modal.state.select(Some(modal.items.len() - 1));
            }
        }
    }

    fn set_status(&mut self, level: MessageLevel, text: impl Into<String>) {
        self.ui.status = Some(StatusLineState {
            level,
            text: text.into(),
        });
    }
}

impl SingleSelectModal {
    fn new(path: ElementPath, spec: SingleSelectSpec) -> Self {
        let mut state = ListState::default();
        let selected = spec
            .initial
            .as_ref()
            .and_then(|initial| {
                spec.options
                    .iter()
                    .position(|option| &option.value == initial)
            })
            .or(Some(0));
        state.select(selected);
        Self { path, spec, state }
    }
}

impl MultiSelectModal {
    fn new(path: ElementPath, spec: MultiSelectSpec) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        let selected = spec.selected.iter().cloned().collect::<BTreeSet<_>>();
        Self {
            path,
            spec,
            selected,
            state,
            error: None,
        }
    }
}

impl InputModal {
    fn new(path: ElementPath, spec: InputPageSpec) -> Self {
        Self {
            path,
            buffer: InputBuffer::new(spec.initial.clone().unwrap_or_default()),
            spec,
            error: None,
        }
    }
}

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
    terminal.hide_cursor().context("failed to hide cursor")?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2).max(1));
    let height = height.min(area.height.saturating_sub(2).max(1));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(height),
            Constraint::Percentage(50),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(width),
            Constraint::Percentage(50),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn list_previous(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let next = match state.selected() {
        Some(0) | None => 0,
        Some(index) => index.saturating_sub(1),
    };
    state.select(Some(next.min(len.saturating_sub(1))));
}

fn list_next(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let next = match state.selected() {
        None => 0,
        Some(index) => (index + 1).min(len.saturating_sub(1)),
    };
    state.select(Some(next));
}

fn ordered_multi_selection(spec: &MultiSelectSpec, selected: &BTreeSet<String>) -> Vec<String> {
    spec.options
        .iter()
        .filter(|option| selected.contains(&option.value))
        .map(|option| option.value.clone())
        .collect()
}

fn kind_for_input(kind: InputKind) -> InputBufferKind {
    match kind {
        InputKind::Text | InputKind::Path | InputKind::Command => InputBufferKind::Text,
        InputKind::Integer => InputBufferKind::Integer,
        InputKind::Number => InputBufferKind::Number,
    }
}

fn apply_single_select_binding(
    app: &mut AppState,
    current_path: &ElementPath,
    binding: &SingleSelectBinding,
    value: Option<String>,
) -> anyhow::Result<()> {
    match binding {
        SingleSelectBinding::SetCurrentString => match value {
            Some(value) => app.set_string(current_path.clone(), value),
            None => app.clear_optional(current_path.clone()),
        },
        SingleSelectBinding::SetString { path } => match value {
            Some(value) => app.set_string(path.clone(), value),
            None => app.clear_optional(path.clone()),
        },
        SingleSelectBinding::SetEnumVariant { path } => match value {
            Some(value) => app.set_enum_variant(path.clone(), &value),
            None => app.clear_optional(path.clone()),
        },
        SingleSelectBinding::Custom(callback) => {
            let mut mutation = crate::data::HookMutation::new(app);
            callback(&mut mutation, value.unwrap_or_default())
        }
    }
}

fn apply_multi_select_binding(
    app: &mut AppState,
    current_path: &ElementPath,
    binding: &MultiSelectBinding,
    values: Vec<String>,
) -> anyhow::Result<()> {
    match binding {
        MultiSelectBinding::SetCurrentStringArray => {
            app.set_string_array(current_path.clone(), values)
        }
        MultiSelectBinding::SetStringArray { path } => app.set_string_array(path.clone(), values),
        MultiSelectBinding::Custom(callback) => {
            let mut mutation = crate::data::HookMutation::new(app);
            callback(&mut mutation, values)
        }
    }
}

fn apply_input_binding(
    app: &mut AppState,
    current_path: &ElementPath,
    spec: &InputPageSpec,
    value: String,
) -> anyhow::Result<()> {
    let trimmed = value.trim().to_string();
    if !spec.allow_empty && trimmed.is_empty() {
        bail!("value cannot be empty");
    }

    match &spec.binding {
        InputBinding::SetCurrentString => app.set_string(current_path.clone(), value),
        InputBinding::SetString { path } => app.set_string(path.clone(), value),
        InputBinding::SetOptionalString { path } => {
            if trimmed.is_empty() {
                app.set_optional_string(path.clone(), None)
            } else {
                app.set_optional_string(path.clone(), Some(value))
            }
        }
        InputBinding::SetInteger { path } => app.set_integer(path.clone(), trimmed.parse::<i64>()?),
        InputBinding::SetOptionalInteger { path } => {
            if trimmed.is_empty() {
                app.set_optional_integer(path.clone(), None)
            } else {
                app.set_optional_integer(path.clone(), Some(trimmed.parse::<i64>()?))
            }
        }
        InputBinding::SetNumber { path } => app.set_number(path.clone(), trimmed.parse::<f64>()?),
        InputBinding::SetOptionalNumber { path } => {
            if trimmed.is_empty() {
                app.set_optional_number(path.clone(), None)
            } else {
                app.set_optional_number(path.clone(), Some(trimmed.parse::<f64>()?))
            }
        }
        InputBinding::Custom(callback) => {
            let mut mutation = crate::data::HookMutation::new(app);
            callback(&mut mutation, value)
        }
    }
}
