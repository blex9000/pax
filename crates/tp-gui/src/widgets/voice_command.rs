use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

const VOICE_METER_BARS: usize = 28;

pub(crate) type WorkspaceContextProvider = Rc<dyn Fn() -> serde_json::Value>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceAction {
    InsertText(String),
    Newline,
    Key(VoiceKey),
    Pax(PaxCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceKey {
    Enter,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Backspace,
    Delete,
    Tab,
    Escape,
    CtrlC,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaxCommand {
    SelectTab(String),
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePlan {
    pub actions: Vec<VoiceAction>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceExecutionReport {
    pub executed: usize,
    pub skipped: Vec<String>,
}

#[derive(Clone, Copy)]
enum MarkerKind {
    InsertText,
    InsertLiteral,
    Keyboard,
    Pax,
    Newline,
}

struct Marker {
    token: &'static str,
    kind: MarkerKind,
}

const MARKERS: &[Marker] = &[
    Marker {
        token: "scrivi letteralmente:",
        kind: MarkerKind::InsertLiteral,
    },
    Marker {
        token: "scrivi:",
        kind: MarkerKind::InsertText,
    },
    Marker {
        token: "tastiera:",
        kind: MarkerKind::Keyboard,
    },
    Marker {
        token: "pax:",
        kind: MarkerKind::Pax,
    },
    Marker {
        token: "va a capo",
        kind: MarkerKind::Newline,
    },
    Marker {
        token: "nuova riga",
        kind: MarkerKind::Newline,
    },
    Marker {
        token: "a capo",
        kind: MarkerKind::Newline,
    },
];

pub fn parse_voice_phrase(phrase: &str) -> VoicePlan {
    let lower = phrase.to_ascii_lowercase();
    let mut pos = 0usize;
    let mut actions = Vec::new();
    let mut issues = Vec::new();

    while pos < phrase.len() {
        let Some((marker_pos, marker)) = find_next_marker(&lower, pos) else {
            let ignored = phrase[pos..].trim();
            if !ignored.is_empty() {
                issues.push(format!("Testo ignorato senza prefisso: {ignored}"));
            }
            break;
        };

        if marker_pos > pos {
            let ignored = phrase[pos..marker_pos].trim();
            if !ignored.is_empty() {
                issues.push(format!("Testo ignorato senza prefisso: {ignored}"));
            }
        }

        let payload_start = marker_pos + marker.token.len();
        match marker.kind {
            MarkerKind::Newline => {
                actions.push(VoiceAction::Newline);
                pos = payload_start;
            }
            MarkerKind::InsertText | MarkerKind::InsertLiteral => {
                let next = find_next_marker(&lower, payload_start)
                    .map(|(idx, _)| idx)
                    .unwrap_or(phrase.len());
                let text = strip_wrapping_quotes(phrase[payload_start..next].trim());
                if text.is_empty() {
                    issues.push("Comando scrivi senza testo".to_string());
                } else {
                    actions.push(VoiceAction::InsertText(text.to_string()));
                }
                pos = next;
            }
            MarkerKind::Keyboard => {
                let next = find_next_marker(&lower, payload_start)
                    .map(|(idx, _)| idx)
                    .unwrap_or(phrase.len());
                let payload = phrase[payload_start..next].trim();
                match parse_key(payload) {
                    Some(key) => actions.push(VoiceAction::Key(key)),
                    None => issues.push(format!("Tasto non riconosciuto: {payload}")),
                }
                pos = next;
            }
            MarkerKind::Pax => {
                let next = find_next_marker(&lower, payload_start)
                    .map(|(idx, _)| idx)
                    .unwrap_or(phrase.len());
                let payload = strip_wrapping_quotes(phrase[payload_start..next].trim());
                if payload.is_empty() {
                    issues.push("Comando pax senza testo".to_string());
                } else {
                    actions.push(VoiceAction::Pax(parse_pax_command(payload)));
                }
                pos = next;
            }
        }
    }

    VoicePlan { actions, issues }
}

pub fn execute_voice_actions(
    panel_type: &str,
    actions: &[VoiceAction],
    writer: &dyn Fn(&[u8]) -> bool,
) -> VoiceExecutionReport {
    let mut executed = 0usize;
    let mut skipped = Vec::new();

    for action in actions {
        match action {
            VoiceAction::InsertText(text) => {
                if writer(text.as_bytes()) {
                    executed += 1;
                } else {
                    skipped.push("Scrittura fallita".to_string());
                }
            }
            VoiceAction::Newline => {
                if panel_type == "terminal" {
                    skipped.push(
                        "Nel terminale 'va a capo' non preme Invio: usa 'tastiera: invio'"
                            .to_string(),
                    );
                } else if writer(b"\n") {
                    executed += 1;
                } else {
                    skipped.push("Nuova riga fallita".to_string());
                }
            }
            VoiceAction::Key(key) => match key_bytes(panel_type, key) {
                Some(bytes) if writer(bytes) => executed += 1,
                Some(_) => skipped.push(format!("Tasto non inviato: {}", key.label())),
                None => skipped.push(format!(
                    "Tasto '{}' non supportato nel pannello {}",
                    key.label(),
                    panel_type
                )),
            },
            VoiceAction::Pax(command) => {
                skipped.push(format!(
                    "Comando Pax non ancora collegato: {}",
                    command.label()
                ));
            }
        }
    }

    VoiceExecutionReport { executed, skipped }
}

pub fn build_voice_command_button(
    panel_type: Rc<dyn Fn() -> Option<String>>,
    writer: Rc<dyn Fn(&[u8]) -> bool>,
) -> gtk4::Button {
    let button = gtk4::Button::new();
    button.set_icon_name("audio-input-microphone-symbolic");
    button.add_css_class("flat");
    button.add_css_class("panel-action-btn");
    button.set_tooltip_text(Some("Voice commands"));

    let tool_executor = Rc::new(|call: crate::voice_tools::VoiceToolCall| {
        crate::voice_tools::VoiceToolExecution::immediate(
            crate::voice_tools::VoiceToolResult::error(&call, "Tool assistant non disponibile"),
        )
    });
    attach_voice_assistant_window(&button, panel_type, writer, tool_executor, None, None, None);

    button
}

pub(crate) fn attach_voice_assistant_window(
    button: &gtk4::Button,
    panel_type: Rc<dyn Fn() -> Option<String>>,
    writer: Rc<dyn Fn(&[u8]) -> bool>,
    tool_executor: Rc<
        dyn Fn(crate::voice_tools::VoiceToolCall) -> crate::voice_tools::VoiceToolExecution,
    >,
    assistant_store: Option<Rc<crate::assistant::AssistantContextStore>>,
    task_supervisor: Option<Rc<crate::assistant_tasks::AssistantTaskSupervisor>>,
    workspace_context: Option<WorkspaceContextProvider>,
) {
    let assistant_window = Rc::new(RefCell::new(None::<gtk4::Window>));
    button.connect_clicked(move |btn| {
        if let Some(window) = assistant_window.borrow().as_ref() {
            window.present();
            return;
        }

        let Some(parent) = btn
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        let window = build_voice_assistant_window(
            &parent,
            panel_type.clone(),
            writer.clone(),
            tool_executor.clone(),
            assistant_store.clone(),
            task_supervisor.clone(),
            workspace_context.clone(),
        );
        window.present();
        assistant_window.borrow_mut().replace(window);
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversationRole {
    User,
    Assistant,
    Tool,
}

impl ConversationRole {
    fn label(self) -> &'static str {
        match self {
            Self::User => "Tu",
            Self::Assistant => "Pax AI",
            Self::Tool => "Azione",
        }
    }

    fn css_class(self) -> &'static str {
        match self {
            Self::User => "assistant-message-user",
            Self::Assistant => "assistant-message-ai",
            Self::Tool => "assistant-message-tool",
        }
    }

    fn persisted_role(self) -> pax_assistant::AssistantRole {
        match self {
            Self::User => pax_assistant::AssistantRole::User,
            Self::Assistant => pax_assistant::AssistantRole::Assistant,
            Self::Tool => pax_assistant::AssistantRole::Tool,
        }
    }
}

struct LiveConversationMessage {
    label: gtk4::Label,
    text: String,
    channel: &'static str,
}

#[derive(Clone)]
struct ConversationFeed {
    chat_list: gtk4::ListBox,
    chat_scroll: gtk4::ScrolledWindow,
    history_list: gtk4::ListBox,
    chat_empty: gtk4::Box,
    history_empty: gtk4::Box,
    assistant_store: Option<Rc<crate::assistant::AssistantContextStore>>,
    live_user: Rc<RefCell<Option<LiveConversationMessage>>>,
    live_assistant: Rc<RefCell<Option<LiveConversationMessage>>>,
}

impl ConversationFeed {
    fn new(
        chat_list: &gtk4::ListBox,
        chat_scroll: &gtk4::ScrolledWindow,
        history_list: &gtk4::ListBox,
        chat_empty: &gtk4::Box,
        history_empty: &gtk4::Box,
        assistant_store: Option<Rc<crate::assistant::AssistantContextStore>>,
    ) -> Self {
        let feed = Self {
            chat_list: chat_list.clone(),
            chat_scroll: chat_scroll.clone(),
            history_list: history_list.clone(),
            chat_empty: chat_empty.clone(),
            history_empty: history_empty.clone(),
            assistant_store,
            live_user: Rc::new(RefCell::new(None)),
            live_assistant: Rc::new(RefCell::new(None)),
        };
        feed.refresh_history();
        feed
    }

    fn append_final(&self, role: ConversationRole, text: &str, channel: &'static str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.chat_empty.set_visible(false);
        append_message_row(&self.chat_list, role, text, None);
        self.scroll_chat_to_bottom();
        self.persist(role, text, channel);
    }

    fn update_live(&self, role: ConversationRole, text: &str, channel: &'static str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.chat_empty.set_visible(false);
        let slot = match role {
            ConversationRole::User => &self.live_user,
            ConversationRole::Assistant => &self.live_assistant,
            ConversationRole::Tool => return,
        };
        let existing_label = {
            let mut live_message = slot.borrow_mut();
            live_message.as_mut().map(|message| {
                message.text = text.to_string();
                message.label.clone()
            })
        };
        if let Some(label) = existing_label {
            label.set_text(text);
        } else {
            let label = append_message_row(&self.chat_list, role, text, None);
            slot.replace(Some(LiveConversationMessage {
                label,
                text: text.to_string(),
                channel,
            }));
        }
        self.scroll_chat_to_bottom();
    }

    fn finalize_live(&self, role: ConversationRole) {
        let slot = match role {
            ConversationRole::User => &self.live_user,
            ConversationRole::Assistant => &self.live_assistant,
            ConversationRole::Tool => return,
        };
        let message = slot.borrow_mut().take();
        if let Some(message) = message {
            self.persist(role, &message.text, message.channel);
        }
    }

    fn persist(&self, role: ConversationRole, text: &str, channel: &'static str) {
        if let Some(store) = self.assistant_store.as_ref() {
            store.append_message(
                role.persisted_role(),
                text,
                &serde_json::json!({ "channel": channel }),
            );
            self.refresh_history();
        }
    }

    fn refresh_history(&self) {
        clear_list_box(&self.history_list);
        let Some(store) = self.assistant_store.as_ref() else {
            self.history_empty.set_visible(true);
            return;
        };
        let messages = store.messages();
        self.history_empty.set_visible(messages.is_empty());
        if messages.is_empty() {
            return;
        }
        for message in messages {
            let role = match message.role.as_str() {
                "user" => ConversationRole::User,
                "assistant" => ConversationRole::Assistant,
                _ => ConversationRole::Tool,
            };
            append_message_row(
                &self.history_list,
                role,
                &message.content,
                Some(message.created_at),
            );
        }
    }

    fn scroll_chat_to_bottom(&self) {
        let scroll = self.chat_scroll.clone();
        gtk4::glib::idle_add_local_once(move || {
            let adjustment = scroll.vadjustment();
            adjustment.set_value((adjustment.upper() - adjustment.page_size()).max(0.0));
        });
    }
}

fn append_message_row(
    list: &gtk4::ListBox,
    role: ConversationRole,
    text: &str,
    created_at: Option<i64>,
) -> gtk4::Label {
    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    row.add_css_class("assistant-message-row");

    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    outer.set_halign(if role == ConversationRole::User {
        gtk4::Align::End
    } else {
        gtk4::Align::Start
    });
    outer.set_hexpand(true);

    let bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    bubble.add_css_class("assistant-message");
    bubble.add_css_class(role.css_class());

    let heading = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let role_label = gtk4::Label::new(Some(role.label()));
    role_label.add_css_class("assistant-message-role");
    role_label.set_xalign(0.0);
    heading.append(&role_label);
    if let Some(created_at) = created_at {
        let time = chrono::DateTime::from_timestamp(created_at, 0)
            .map(|value| value.with_timezone(&chrono::Local))
            .map(|value| value.format("%d/%m %H:%M").to_string())
            .unwrap_or_default();
        let time_label = gtk4::Label::new(Some(&time));
        time_label.add_css_class("assistant-message-time");
        heading.append(&time_label);
    }
    bubble.append(&heading);

    let content = gtk4::Label::new(Some(text));
    content.add_css_class("assistant-message-content");
    content.set_wrap(true);
    content.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    content.set_max_width_chars(58);
    content.set_selectable(true);
    content.set_xalign(0.0);
    bubble.append(&content);

    outer.append(&bubble);
    row.set_child(Some(&outer));
    list.append(&row);
    content
}

fn clear_list_box(list: &gtk4::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn build_assistant_empty_state(
    icon_name: &str,
    title: &str,
    detail: Option<&str>,
    css_class: &str,
) -> (gtk4::Box, Option<gtk4::Label>) {
    let empty = gtk4::Box::new(gtk4::Orientation::Vertical, 7);
    empty.add_css_class("assistant-empty-state");
    empty.add_css_class(css_class);
    empty.set_halign(gtk4::Align::Center);
    empty.set_valign(gtk4::Align::Center);
    empty.set_can_target(false);

    let icon = gtk4::Image::from_icon_name(icon_name);
    icon.add_css_class("assistant-empty-icon");
    empty.append(&icon);

    let title = gtk4::Label::new(Some(title));
    title.add_css_class("assistant-empty-title");
    empty.append(&title);

    let detail_label = detail.map(|detail| {
        let label = gtk4::Label::new(Some(detail));
        label.add_css_class("assistant-empty-detail");
        label.set_justify(gtk4::Justification::Center);
        label.set_wrap(true);
        label.set_max_width_chars(42);
        empty.append(&label);
        label
    });

    (empty, detail_label)
}

fn assistant_panel_context(panel_type: Option<String>) -> String {
    let panel = match panel_type.as_deref() {
        Some("terminal") => "Terminale",
        Some("markdown") => "Editor Markdown",
        Some("code_editor") => "Editor di codice",
        Some("docker_help") => "Docker",
        Some("note") => "Note",
        Some("empty") => "Pannello vuoto",
        Some(other) => other,
        None => return "Nessun pannello in focus".to_string(),
    };
    format!("Focus: {panel}")
}

fn refresh_assistant_panel_context(
    status_label: &gtk4::Label,
    empty_label: &gtk4::Label,
    panel_type: Option<String>,
) {
    let context = assistant_panel_context(panel_type);
    status_label.set_text(&context);
    empty_label.set_text(&context);
}

fn build_assistant_voice_picker() -> (gtk4::MenuButton, gtk4::DropDown, gtk4::Popover) {
    let current_voice = crate::voice_settings::load_gemini_voice();
    let button = gtk4::MenuButton::new();
    button.set_icon_name("audio-speakers-symbolic");
    button.add_css_class("flat");
    button.add_css_class("assistant-voice-picker");
    button.set_tooltip_text(Some(&format!(
        "Voce AI: {}",
        crate::voice_settings::gemini_voice_selection_label(current_voice.as_deref())
    )));

    let popover = gtk4::Popover::new();
    crate::theme::configure_popover(&popover);
    popover.add_css_class("assistant-voice-popover");

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 7);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);

    let label = gtk4::Label::new(Some("Voce AI"));
    label.add_css_class("heading");
    label.set_xalign(0.0);
    content.append(&label);

    let labels = crate::voice_settings::gemini_voice_labels();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let dropdown = gtk4::DropDown::from_strings(&label_refs);
    dropdown.add_css_class("assistant-voice-dropdown");
    dropdown.set_selected(crate::voice_settings::gemini_voice_index(
        current_voice.as_deref(),
    ));
    content.append(&dropdown);

    {
        let button = button.clone();
        let dropdown = dropdown.clone();
        popover.connect_show(move |_| {
            let current_voice = crate::voice_settings::load_gemini_voice();
            dropdown.set_selected(crate::voice_settings::gemini_voice_index(
                current_voice.as_deref(),
            ));
            button.set_tooltip_text(Some(&format!(
                "Voce AI: {}",
                crate::voice_settings::gemini_voice_selection_label(current_voice.as_deref())
            )));
        });
    }
    popover.set_child(Some(&content));
    button.set_popover(Some(&popover));
    (button, dropdown, popover)
}

fn rebuild_assistant_task_tray(
    tray: &gtk4::Box,
    supervisor: &Rc<crate::assistant_tasks::AssistantTaskSupervisor>,
) {
    while let Some(child) = tray.first_child() {
        tray.remove(&child);
    }
    let tasks = supervisor
        .tasks()
        .into_iter()
        .filter(|task| task.state.is_active())
        .collect::<Vec<_>>();
    tray.set_visible(!tasks.is_empty());

    for task in tasks {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.add_css_class("assistant-task-row");

        let spinner = gtk4::Spinner::new();
        spinner.add_css_class("assistant-task-spinner");
        spinner.start();
        row.append(&spinner);

        let copy = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
        copy.set_hexpand(true);
        let title = gtk4::Label::new(Some(&task.label));
        title.add_css_class("assistant-task-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        copy.append(&title);

        let elapsed = now_millis().saturating_sub(task.created_at_ms).max(0) / 1_000;
        let detail = gtk4::Label::new(Some(&format!(
            "{} · {:02}:{:02}",
            task.target_panel_id.as_deref().unwrap_or("workspace"),
            elapsed / 60,
            elapsed % 60
        )));
        detail.add_css_class("assistant-task-detail");
        detail.set_xalign(0.0);
        copy.append(&detail);
        row.append(&copy);

        let cancel = gtk4::Button::from_icon_name("process-stop-symbolic");
        cancel.add_css_class("flat");
        cancel.add_css_class("assistant-task-cancel");
        cancel.set_tooltip_text(Some("Annulla monitoraggio"));
        let supervisor = supervisor.clone();
        let task_id = task.id.clone();
        cancel.connect_clicked(move |_| {
            supervisor.cancel(&task_id);
        });
        row.append(&cancel);
        tray.append(&row);
    }
}

fn assistant_task_host_event(task: &pax_assistant::AssistantTask) -> String {
    pax_assistant::ProviderTaskAdapter::for_provider(&task.provider)
        .completion_event(task)
        .to_string()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn build_voice_assistant_window(
    parent: &gtk4::Window,
    panel_type: Rc<dyn Fn() -> Option<String>>,
    writer: Rc<dyn Fn(&[u8]) -> bool>,
    tool_executor: Rc<
        dyn Fn(crate::voice_tools::VoiceToolCall) -> crate::voice_tools::VoiceToolExecution,
    >,
    assistant_store: Option<Rc<crate::assistant::AssistantContextStore>>,
    task_supervisor: Option<Rc<crate::assistant_tasks::AssistantTaskSupervisor>>,
    workspace_context: Option<WorkspaceContextProvider>,
) -> gtk4::Window {
    let window = gtk4::Window::builder()
        .title("AI Assistant")
        .transient_for(parent)
        .modal(false)
        .default_width(520)
        .default_height(620)
        .build();
    window.set_destroy_with_parent(true);
    window.set_hide_on_close(true);
    crate::theme::configure_dialog_window(&window);
    window.add_css_class("voice-assistant-window");

    let initial_panel_context = assistant_panel_context(panel_type());
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("voice-assistant-root");
    root.set_size_request(420, 480);

    let transcribe_status = crate::voice_session::resolve_transcribe_status();
    let top_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    top_bar.add_css_class("assistant-top-bar");
    let status_mark = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    status_mark.add_css_class("assistant-status-mark");
    status_mark.set_size_request(32, 32);
    status_mark.set_margin_start(12);
    status_mark.set_margin_top(10);
    status_mark.set_margin_bottom(10);
    let status_icon = gtk4::Image::from_icon_name("starred-symbolic");
    status_icon.add_css_class("assistant-status-icon");
    status_icon.set_margin_start(7);
    status_icon.set_margin_end(7);
    status_icon.set_halign(gtk4::Align::Center);
    status_icon.set_valign(gtk4::Align::Center);
    status_mark.append(&status_icon);
    top_bar.append(&status_mark);

    let status_copy = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
    status_copy.set_hexpand(true);
    status_copy.set_valign(gtk4::Align::Center);
    let status = gtk4::Label::new(Some(transcribe_status.message));
    status.add_css_class("voice-status");
    status.set_wrap(true);
    status.set_xalign(0.0);
    status_copy.append(&status);
    let panel_context_status = gtk4::Label::new(Some(&initial_panel_context));
    panel_context_status.add_css_class("assistant-context-status");
    panel_context_status.set_xalign(0.0);
    status_copy.append(&panel_context_status);
    top_bar.append(&status_copy);
    let (voice_picker, voice_dropdown, voice_popover) = build_assistant_voice_picker();
    voice_picker.set_valign(gtk4::Align::Center);
    voice_picker.set_margin_end(12);
    top_bar.append(&voice_picker);
    root.append(&top_bar);

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(140);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    let chat_list = gtk4::ListBox::new();
    chat_list.add_css_class("assistant-conversation-list");
    chat_list.set_selection_mode(gtk4::SelectionMode::None);
    let chat_scroll = gtk4::ScrolledWindow::new();
    chat_scroll.add_css_class("assistant-conversation-scroll");
    chat_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    chat_scroll.set_vexpand(true);
    chat_scroll.set_child(Some(&chat_list));
    let chat_page = gtk4::Overlay::new();
    chat_page.set_child(Some(&chat_scroll));
    let (chat_empty, chat_context_label) = build_assistant_empty_state(
        "starred-symbolic",
        "Pax AI",
        Some(&initial_panel_context),
        "assistant-chat-empty",
    );
    let chat_context_label = chat_context_label.expect("chat empty state has context");
    chat_page.add_overlay(&chat_empty);
    stack.add_titled(&chat_page, Some("chat"), "Chat");

    let history_list = gtk4::ListBox::new();
    history_list.add_css_class("assistant-conversation-list");
    history_list.add_css_class("assistant-history-list");
    history_list.set_selection_mode(gtk4::SelectionMode::None);
    let history_scroll = gtk4::ScrolledWindow::new();
    history_scroll.add_css_class("assistant-conversation-scroll");
    history_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    history_scroll.set_vexpand(true);
    history_scroll.set_child(Some(&history_list));
    let history_page = gtk4::Overlay::new();
    history_page.set_child(Some(&history_scroll));
    let (history_empty, _) = build_assistant_empty_state(
        "document-open-recent-symbolic",
        "Nessuna conversazione salvata",
        None,
        "assistant-history-empty",
    );
    history_page.add_overlay(&history_empty);
    stack.add_titled(&history_page, Some("history"), "Storico");

    let switcher = gtk4::StackSwitcher::new();
    switcher.add_css_class("assistant-view-switcher");
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk4::Align::Center);
    root.append(&switcher);

    let task_tray = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    task_tray.add_css_class("assistant-task-tray");
    task_tray.set_visible(false);
    root.append(&task_tray);
    root.append(&stack);

    if let Some(supervisor) = task_supervisor.as_ref() {
        rebuild_assistant_task_tray(&task_tray, supervisor);
        let tray_weak = task_tray.downgrade();
        let supervisor_weak = Rc::downgrade(supervisor);
        supervisor.subscribe(Rc::new(move |_| {
            if let (Some(tray), Some(supervisor)) = (tray_weak.upgrade(), supervisor_weak.upgrade())
            {
                rebuild_assistant_task_tray(&tray, &supervisor);
            }
        }));

        let tray_weak = task_tray.downgrade();
        let supervisor_weak = Rc::downgrade(supervisor);
        gtk4::glib::timeout_add_local(Duration::from_secs(1), move || {
            let (Some(tray), Some(supervisor)) = (tray_weak.upgrade(), supervisor_weak.upgrade())
            else {
                return gtk4::glib::ControlFlow::Break;
            };
            rebuild_assistant_task_tray(&tray, &supervisor);
            gtk4::glib::ControlFlow::Continue
        });
    }

    let conversation = ConversationFeed::new(
        &chat_list,
        &chat_scroll,
        &history_list,
        &chat_empty,
        &history_empty,
        assistant_store,
    );

    let (audio_meter, audio_levels) = build_audio_meter();
    audio_meter.set_content_height(18);
    audio_meter.add_css_class("assistant-compact-meter");
    let voice_strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    voice_strip.add_css_class("assistant-voice-strip");
    voice_strip.set_visible(false);
    let voice_icon = gtk4::Image::from_icon_name("media-record-symbolic");
    voice_icon.add_css_class("assistant-voice-live-icon");
    voice_icon.set_tooltip_text(Some("Microfono attivo"));
    voice_strip.append(&voice_icon);
    voice_strip.append(&audio_meter);
    root.append(&voice_strip);

    let composer = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    composer.add_css_class("assistant-composer");
    let message_entry = gtk4::Entry::new();
    message_entry.add_css_class("assistant-message-entry");
    message_entry.set_placeholder_text(Some("Scrivi a Pax AI"));
    message_entry.set_hexpand(true);
    composer.append(&message_entry);

    let send_btn = gtk4::Button::from_icon_name("mail-send-symbolic");
    send_btn.add_css_class("flat");
    send_btn.add_css_class("assistant-control-button");
    send_btn.add_css_class("assistant-send-button");
    send_btn.set_tooltip_text(Some("Invia messaggio"));
    send_btn.set_sensitive(false);
    composer.append(&send_btn);

    let mic_btn = gtk4::ToggleButton::new();
    mic_btn.set_icon_name("audio-input-microphone-symbolic");
    mic_btn.add_css_class("flat");
    mic_btn.add_css_class("assistant-control-button");
    mic_btn.add_css_class("voice-mic-button");
    mic_btn.set_sensitive(transcribe_status.ready);
    mic_btn.set_tooltip_text(Some(transcribe_status.tooltip));
    composer.append(&mic_btn);

    let mute_btn = gtk4::ToggleButton::new();
    mute_btn.set_icon_name("audio-volume-high-symbolic");
    mute_btn.add_css_class("flat");
    mute_btn.add_css_class("assistant-control-button");
    mute_btn.add_css_class("assistant-mute-button");
    mute_btn.set_tooltip_text(Some("Silenzia la voce AI"));
    composer.append(&mute_btn);
    root.append(&composer);

    let current_job: Rc<RefCell<Option<crate::voice_session::VoiceSessionJob>>> =
        Rc::new(RefCell::new(None));
    let queued_host_events = Rc::new(RefCell::new(VecDeque::<String>::new()));
    let job_generation = Rc::new(Cell::new(0u64));
    let suppress_toggle = Rc::new(Cell::new(false));

    if let Some(supervisor) = task_supervisor.as_ref() {
        let current_job = current_job.clone();
        let queued_host_events = queued_host_events.clone();
        let mic_btn = mic_btn.clone();
        let window_weak = window.downgrade();
        let ready = transcribe_status.ready;
        supervisor.subscribe(Rc::new(move |event| {
            let pax_assistant::AssistantTaskEvent::DeliveryRequired(task) = event else {
                return;
            };
            let event = assistant_task_host_event(&task);
            if current_job
                .borrow()
                .as_ref()
                .is_some_and(|job| job.send_host_event(&event).is_ok())
            {
                return;
            }
            queued_host_events.borrow_mut().push_back(event);
            if ready
                && window_weak
                    .upgrade()
                    .is_some_and(|window| window.is_visible())
                && current_job.borrow().is_none()
            {
                mic_btn.set_active(true);
                mic_btn.set_active(false);
            }
        }));
    }

    {
        let current_job = current_job.clone();
        let job_generation = job_generation.clone();
        let mic_btn = mic_btn.clone();
        let suppress_toggle = suppress_toggle.clone();
        let status = status.clone();
        let voice_strip = voice_strip.clone();
        let voice_picker = voice_picker.clone();
        let voice_popover = voice_popover.clone();
        voice_dropdown.connect_selected_notify(move |dropdown| {
            let selected = crate::voice_settings::gemini_voice_at_index(dropdown.selected());
            let current = crate::voice_settings::load_gemini_voice();
            if current.as_deref() == selected {
                return;
            }

            crate::voice_settings::save_gemini_voice(selected.unwrap_or_default());
            let label = crate::voice_settings::gemini_voice_selection_label(selected);
            voice_picker.set_tooltip_text(Some(&format!("Voce AI: {label}")));
            voice_popover.popdown();

            let was_listening = mic_btn.is_active();
            job_generation.set(job_generation.get().wrapping_add(1));
            if let Some(job) = current_job.borrow_mut().take() {
                job.cancel();
            }
            set_toggle_active_silently(&mic_btn, &suppress_toggle, false);
            voice_strip.set_visible(false);

            if was_listening {
                status.set_text(&format!("Riavvio Gemini Live con la voce {label}..."));
                mic_btn.set_active(true);
            } else {
                status.set_text(&format!("Voce {label} selezionata."));
            }
        });
    }

    {
        let current_job = current_job.clone();
        let job_generation = job_generation.clone();
        let suppress_toggle = suppress_toggle.clone();
        let provider = transcribe_status.provider.clone();
        let status = status.clone();
        let panel_context_status = panel_context_status.clone();
        let chat_context_label = chat_context_label.clone();
        let audio_meter = audio_meter.clone();
        let audio_levels = audio_levels.clone();
        let voice_strip = voice_strip.clone();
        let conversation_for_voice = conversation.clone();
        let mute_btn_for_session = mute_btn.clone();
        let panel_type_for_voice = panel_type.clone();
        let writer_for_voice = writer.clone();
        let tool_executor_for_voice = tool_executor.clone();
        let workspace_context_for_voice = workspace_context.clone();
        let queued_host_events_for_voice = queued_host_events.clone();
        mic_btn.connect_toggled(move |btn| {
            if suppress_toggle.get() {
                return;
            }

            if !btn.is_active() {
                voice_strip.set_visible(false);
                if let Some(job) = current_job.borrow().as_ref() {
                    if let Err(error) = job.set_microphone_enabled(false) {
                        status.set_text(&error);
                    } else {
                        status.set_text("Microfono disattivato.");
                    }
                } else {
                    status.set_text("Microfono disattivato.");
                }
                return;
            }

            refresh_assistant_panel_context(
                &panel_context_status,
                &chat_context_label,
                panel_type_for_voice(),
            );
            voice_strip.set_visible(true);
            if let Some(job) = current_job.borrow().as_ref() {
                if let Err(error) = job.set_microphone_enabled(true) {
                    status.set_text(&error);
                } else {
                    status.set_text("In ascolto...");
                }
                return;
            }

            let Some(provider) = provider.clone() else {
                set_toggle_active_silently(btn, &suppress_toggle, false);
                status.set_text("Provider voce non trovato.");
                conversation_for_voice.append_final(
                    ConversationRole::Tool,
                    "Provider Gemini non incluso in questa build.",
                    "system",
                );
                return;
            };

            job_generation.set(job_generation.get().wrapping_add(1));
            let token = job_generation.get();
            status.set_text("Avvio Gemini Live...");
            reset_audio_meter(&audio_levels, &audio_meter);

            let btn_c = btn.clone();
            let current_job_c = current_job.clone();
            let job_generation_c = job_generation.clone();
            let suppress_toggle_c = suppress_toggle.clone();
            let status_c = status.clone();
            let job_generation_level = job_generation.clone();
            let job_generation_status = job_generation.clone();
            let job_generation_partial = job_generation.clone();
            let job_generation_response = job_generation.clone();
            let job_generation_command = job_generation.clone();
            let job_generation_turn = job_generation.clone();
            let audio_meter_c = audio_meter.clone();
            let audio_levels_c = audio_levels.clone();
            let audio_meter_done = audio_meter.clone();
            let audio_levels_done = audio_levels.clone();
            let voice_strip_done = voice_strip.clone();
            let status_progress = status.clone();
            let status_response = status.clone();
            let status_command = status.clone();
            let status_tool = status.clone();
            let conversation_partial = conversation_for_voice.clone();
            let conversation_response = conversation_for_voice.clone();
            let conversation_command = conversation_for_voice.clone();
            let conversation_tool = conversation_for_voice.clone();
            let conversation_turn = conversation_for_voice.clone();
            let conversation_done = conversation_for_voice.clone();
            let panel_type_exec = panel_type_for_voice.clone();
            let writer_exec = writer_for_voice.clone();
            let tool_executor_exec = tool_executor_for_voice.clone();
            let context = crate::voice_provider::VoiceContext {
                panel_type: panel_type_for_voice(),
                workspace: workspace_context_for_voice
                    .as_ref()
                    .map(|provide_context| provide_context()),
            };

            match start_transcribe_command(
                provider,
                context,
                move |level| {
                    if job_generation_level.get() != token {
                        return;
                    }
                    push_audio_level(&audio_levels_c, &audio_meter_c, level);
                },
                move |message| {
                    if job_generation_status.get() != token {
                        return;
                    }
                    status_progress.set_text(&message);
                },
                move |partial| {
                    if job_generation_partial.get() == token {
                        conversation_partial.update_live(ConversationRole::User, &partial, "voice");
                    }
                },
                move |response| {
                    if job_generation_response.get() == token {
                        status_response.set_text("Gemini sta rispondendo...");
                        conversation_response.finalize_live(ConversationRole::User);
                        conversation_response.update_live(
                            ConversationRole::Assistant,
                            &response,
                            "voice",
                        );
                    }
                },
                move |result| {
                    if job_generation_command.get() != token {
                        return;
                    }
                    let command = result.command.trim().to_string();
                    if let Some(transcript) = result.transcript.as_deref() {
                        conversation_command.update_live(
                            ConversationRole::User,
                            transcript,
                            "voice",
                        );
                    }
                    conversation_command.finalize_live(ConversationRole::User);
                    let plan = parse_voice_phrase(&command);
                    if plan.actions.is_empty() {
                        status_command
                            .set_text("Comando Live riconosciuto, ma nessuna azione valida.");
                        conversation_command.append_final(
                            ConversationRole::Tool,
                            "La richiesta non ha prodotto azioni eseguibili.",
                            "tool",
                        );
                        return;
                    }

                    let panel_type = panel_type_exec().unwrap_or_else(|| "unknown".to_string());
                    let report =
                        execute_voice_actions(&panel_type, &plan.actions, writer_exec.as_ref());
                    status_command.set_text("Comando eseguito. Gemini Live resta in ascolto.");
                    conversation_command.append_final(
                        ConversationRole::Tool,
                        &execution_preview(&plan, &report),
                        "tool",
                    );
                },
                move |call| {
                    conversation_tool.finalize_live(ConversationRole::User);
                    match tool_executor_exec(call) {
                        crate::voice_tools::VoiceToolExecution::Immediate(result) => {
                            let failed = result
                                .response
                                .get("status")
                                .and_then(serde_json::Value::as_str)
                                == Some("error");
                            status_tool.set_text(if failed {
                                "Operazione fallita. Gemini chiarira' il problema."
                            } else {
                                "Operazione completata. Gemini Live resta in ascolto."
                            });
                            conversation_tool.append_final(
                                ConversationRole::Tool,
                                &voice_tool_preview(&result),
                                "tool",
                            );
                            crate::voice_tools::VoiceToolExecution::Immediate(result)
                        }
                        pending @ crate::voice_tools::VoiceToolExecution::Pending { .. } => {
                            status_tool
                                .set_text("Operazione in corso. Pax controllera' automaticamente.");
                            pending
                        }
                    }
                },
                move || {
                    if job_generation_turn.get() == token {
                        conversation_turn.finalize_live(ConversationRole::User);
                        conversation_turn.finalize_live(ConversationRole::Assistant);
                    }
                },
                move |result| {
                    if job_generation_c.get() != token {
                        return;
                    }

                    current_job_c.borrow_mut().take();
                    set_toggle_active_silently(&btn_c, &suppress_toggle_c, false);
                    voice_strip_done.set_visible(false);
                    conversation_done.finalize_live(ConversationRole::User);
                    conversation_done.finalize_live(ConversationRole::Assistant);

                    match result {
                        TranscribeResult::Completed => {
                            status_c.set_text("Sessione Gemini Live terminata.");
                            reset_audio_meter(&audio_levels_done, &audio_meter_done);
                        }
                        TranscribeResult::Cancelled => {
                            status_c.set_text("Sessione Gemini Live chiusa.");
                            reset_audio_meter(&audio_levels_done, &audio_meter_done);
                        }
                        TranscribeResult::Failed(err) => {
                            status_c.set_text("Sessione Gemini Live fallita.");
                            conversation_done.append_final(ConversationRole::Tool, &err, "system");
                        }
                    }
                },
            ) {
                Ok(job) => {
                    let _ = job.set_microphone_enabled(true);
                    let _ = job.set_output_muted(mute_btn_for_session.is_active());
                    loop {
                        let event = queued_host_events_for_voice.borrow_mut().pop_front();
                        let Some(event) = event else {
                            break;
                        };
                        if let Err(error) = job.send_host_event(&event) {
                            queued_host_events_for_voice.borrow_mut().push_front(event);
                            status.set_text(&error);
                            break;
                        }
                    }
                    *current_job.borrow_mut() = Some(job);
                }
                Err(err) => {
                    set_toggle_active_silently(btn, &suppress_toggle, false);
                    status.set_text("Sessione non avviata.");
                    conversation_for_voice.append_final(ConversationRole::Tool, &err, "system");
                }
            }
        });
    }

    {
        let current_job = current_job.clone();
        let queued_host_events = queued_host_events.clone();
        let mic_btn = mic_btn.clone();
        let ready = transcribe_status.ready;
        window.connect_show(move |_| {
            if ready && current_job.borrow().is_none() && !queued_host_events.borrow().is_empty() {
                mic_btn.set_active(true);
                mic_btn.set_active(false);
            }
        });
    }

    {
        let current_job = current_job.clone();
        let status = status.clone();
        mute_btn.connect_toggled(move |btn| {
            let muted = btn.is_active();
            btn.set_icon_name(if muted {
                "audio-volume-muted-symbolic"
            } else {
                "audio-volume-high-symbolic"
            });
            btn.set_tooltip_text(Some(if muted {
                "Riattiva la voce AI"
            } else {
                "Silenzia la voce AI"
            }));
            if let Some(job) = current_job.borrow().as_ref() {
                if let Err(error) = job.set_output_muted(muted) {
                    status.set_text(&error);
                }
            }
        });
    }

    let send_message: Rc<dyn Fn()> = {
        let current_job = current_job.clone();
        let mic_btn = mic_btn.clone();
        let message_entry = message_entry.clone();
        let conversation = conversation.clone();
        let status = status.clone();
        let panel_context_status = panel_context_status.clone();
        let chat_context_label = chat_context_label.clone();
        let panel_type = panel_type.clone();
        let ready = transcribe_status.ready;
        Rc::new(move || {
            let text = message_entry.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            if !ready {
                status.set_text("Gemini API key mancante.");
                return;
            }

            refresh_assistant_panel_context(
                &panel_context_status,
                &chat_context_label,
                panel_type(),
            );
            conversation.finalize_live(ConversationRole::User);
            conversation.append_final(ConversationRole::User, &text, "text");

            let started_for_text = current_job.borrow().is_none();
            if started_for_text {
                mic_btn.set_active(true);
                mic_btn.set_active(false);
            }
            let result = current_job
                .borrow()
                .as_ref()
                .ok_or_else(|| "Sessione Gemini Live non disponibile.".to_string())
                .and_then(|job| job.send_text(&text));
            match result {
                Ok(()) => {
                    message_entry.set_text("");
                    status.set_text("Gemini sta elaborando...");
                }
                Err(error) => {
                    status.set_text(&error);
                    conversation.append_final(ConversationRole::Tool, &error, "system");
                }
            }
        })
    };

    {
        let send_message = send_message.clone();
        send_btn.connect_clicked(move |_| send_message());
    }
    {
        let send_message = send_message.clone();
        message_entry.connect_activate(move |_| send_message());
    }
    {
        let send_btn = send_btn.clone();
        let ready = transcribe_status.ready;
        message_entry.connect_changed(move |entry| {
            send_btn.set_sensitive(ready && !entry.text().trim().is_empty());
        });
    }

    {
        let current_job = current_job.clone();
        let job_generation = job_generation.clone();
        let mic_btn = mic_btn.clone();
        let suppress_toggle = suppress_toggle.clone();
        let status = status.clone();
        let voice_strip = voice_strip.clone();
        window.connect_hide(move |_| {
            job_generation.set(job_generation.get().wrapping_add(1));
            if let Some(job) = current_job.borrow_mut().take() {
                job.cancel();
            }
            set_toggle_active_silently(&mic_btn, &suppress_toggle, false);
            voice_strip.set_visible(false);
            status.set_text("Sessione chiusa.");
        });
    }

    window.set_child(Some(&root));
    window
}

fn set_toggle_active_silently(
    button: &gtk4::ToggleButton,
    suppress_toggle: &Cell<bool>,
    active: bool,
) {
    suppress_toggle.set(true);
    button.set_active(active);
    suppress_toggle.set(false);
}

fn build_audio_meter() -> (gtk4::DrawingArea, Rc<RefCell<VecDeque<f64>>>) {
    let levels = Rc::new(RefCell::new(VecDeque::from(vec![0.0; VOICE_METER_BARS])));
    let area = gtk4::DrawingArea::new();
    area.add_css_class("voice-audio-meter");
    area.set_content_width(230);
    area.set_content_height(42);
    area.set_hexpand(true);

    let levels_for_draw = levels.clone();
    area.set_draw_func(move |_, cr, width, height| {
        draw_audio_meter(cr, width, height, &levels_for_draw.borrow());
    });

    (area, levels)
}

fn push_audio_level(levels: &Rc<RefCell<VecDeque<f64>>>, area: &gtk4::DrawingArea, level: f64) {
    let mut levels = levels.borrow_mut();
    if levels.len() >= VOICE_METER_BARS {
        levels.pop_front();
    }
    levels.push_back(level.clamp(0.0, 1.0));
    area.queue_draw();
}

fn reset_audio_meter(levels: &Rc<RefCell<VecDeque<f64>>>, area: &gtk4::DrawingArea) {
    let mut levels = levels.borrow_mut();
    levels.clear();
    for _ in 0..VOICE_METER_BARS {
        levels.push_back(0.0);
    }
    area.queue_draw();
}

fn draw_audio_meter(cr: &gtk4::cairo::Context, width: i32, height: i32, levels: &VecDeque<f64>) {
    let width = width.max(1) as f64;
    let height = height.max(1) as f64;
    let radius = 6.0;
    rounded_rect(cr, 0.5, 0.5, width - 1.0, height - 1.0, radius);
    cr.set_source_rgba(0.08, 0.11, 0.16, 0.22);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(0.35, 0.42, 0.52, 0.35);
    let _ = cr.stroke();

    let padding = 6.0;
    let gap = 2.0;
    let bar_count = VOICE_METER_BARS as f64;
    let available_width = (width - padding * 2.0 - gap * (bar_count - 1.0)).max(1.0);
    let bar_width = (available_width / bar_count).max(2.0);
    let max_height = (height - padding * 2.0).max(1.0);

    for (idx, level) in levels.iter().enumerate() {
        let level = level.clamp(0.0, 1.0);
        let bar_height = (max_height * (0.08 + level * 0.92)).max(2.0);
        let x = padding + idx as f64 * (bar_width + gap);
        let y = padding + (max_height - bar_height) / 2.0;
        rounded_rect(cr, x, y, bar_width, bar_height, bar_width.min(4.0) / 2.0);
        cr.set_source_rgba(0.18, 0.58, 0.96, 0.35 + level * 0.55);
        let _ = cr.fill();
    }
}

fn rounded_rect(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        std::f64::consts::PI * 1.5,
    );
    cr.close_path();
}

enum TranscribeResult {
    Completed,
    Cancelled,
    Failed(String),
}

fn start_transcribe_command(
    provider: crate::voice_session::VoiceProvider,
    context: crate::voice_provider::VoiceContext,
    on_audio_level: impl Fn(f64) + 'static,
    on_status: impl Fn(String) + 'static,
    on_partial_transcript: impl Fn(String) + 'static,
    on_assistant_transcript: impl Fn(String) + 'static,
    on_command: impl Fn(crate::voice_provider::GeminiTranscript) + 'static,
    on_tool_call: impl Fn(crate::voice_tools::VoiceToolCall) -> crate::voice_tools::VoiceToolExecution
        + 'static,
    on_turn_complete: impl Fn() + 'static,
    on_done: impl FnOnce(TranscribeResult) + 'static,
) -> Result<crate::voice_session::VoiceSessionJob, String> {
    let (job, receiver) = crate::voice_session::start_transcribe_session(provider, context)?;
    let callback = Rc::new(RefCell::new(Some(on_done)));
    gtk4::glib::timeout_add_local(Duration::from_millis(16), move || {
        let dispatch =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || match drain_transcribe_events(
                    &receiver,
                    &on_audio_level,
                    &on_status,
                    &on_partial_transcript,
                    &on_assistant_transcript,
                    &on_command,
                    &on_tool_call,
                    &on_turn_complete,
                ) {
                    Some(result) => {
                        let done = callback.borrow_mut().take();
                        if let Some(done) = done {
                            done(result);
                        }
                        gtk4::glib::ControlFlow::Break
                    }
                    None => gtk4::glib::ControlFlow::Continue,
                },
            ));

        match dispatch {
            Ok(flow) => flow,
            Err(payload) => {
                let error = panic_payload_message(payload.as_ref());
                tracing::error!(%error, "AI Assistant event dispatch panicked");
                let done = callback.borrow_mut().take();
                if let Some(done) = done {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        done(TranscribeResult::Failed(format!(
                            "Errore interno AI Assistant: {error}"
                        )));
                    }));
                }
                gtk4::glib::ControlFlow::Break
            }
        }
    });

    Ok(job)
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else {
        "panic senza messaggio".to_string()
    }
}

fn drain_transcribe_events(
    receiver: &Receiver<crate::voice_session::VoiceSessionEvent>,
    on_audio_level: &impl Fn(f64),
    on_status: &impl Fn(String),
    on_partial_transcript: &impl Fn(String),
    on_assistant_transcript: &impl Fn(String),
    on_command: &impl Fn(crate::voice_provider::GeminiTranscript),
    on_tool_call: &impl Fn(crate::voice_tools::VoiceToolCall) -> crate::voice_tools::VoiceToolExecution,
    on_turn_complete: &impl Fn(),
) -> Option<TranscribeResult> {
    let mut done = None;
    while let Ok(event) = receiver.try_recv() {
        match event {
            crate::voice_session::VoiceSessionEvent::AudioLevel(level) => on_audio_level(level),
            crate::voice_session::VoiceSessionEvent::Status(message) => on_status(message),
            crate::voice_session::VoiceSessionEvent::PartialTranscript(transcript) => {
                on_partial_transcript(transcript);
            }
            crate::voice_session::VoiceSessionEvent::AssistantTranscript(transcript) => {
                on_assistant_transcript(transcript);
            }
            crate::voice_session::VoiceSessionEvent::Command(result) => {
                on_command(result);
            }
            crate::voice_session::VoiceSessionEvent::ToolCall { call, response } => {
                let _ = response.send(on_tool_call(call));
            }
            crate::voice_session::VoiceSessionEvent::TurnComplete => on_turn_complete(),
            crate::voice_session::VoiceSessionEvent::Completed => {
                done = Some(TranscribeResult::Completed);
            }
            crate::voice_session::VoiceSessionEvent::Cancelled => {
                done = Some(TranscribeResult::Cancelled);
            }
            crate::voice_session::VoiceSessionEvent::Failed(err) => {
                done = Some(TranscribeResult::Failed(err));
            }
        }
    }
    done
}

fn voice_tool_preview(result: &crate::voice_tools::VoiceToolResult) -> String {
    let response = &result.response;
    if let Some(error) = response.get("error").and_then(serde_json::Value::as_str) {
        return error.to_string();
    }
    match result.name.as_str() {
        crate::voice_tools::WORKSPACE_INSPECT_TOOL => format!(
            "Workspace ispezionato: {} pannelli.",
            response
                .pointer("/workspace/panels")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        ),
        crate::voice_tools::WORKSPACE_SELECT_TAB_TOOL => format!(
            "Tab '{}' selezionato.",
            response
                .pointer("/result/selected_tab")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("-")
        ),
        crate::voice_tools::WORKSPACE_ACTION_TOOL => {
            let action = response
                .pointer("/result/action")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("workspace_action");
            format!("Azione workspace completata: {action}.")
        }
        crate::voice_tools::TERMINAL_READ_TOOL => format!(
            "Terminale letto: {} righe recenti.",
            response
                .get("returned_lines")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        ),
        crate::voice_tools::TERMINAL_WRITE_TOOL => format!(
            "Testo inviato al terminale: {} caratteri.",
            response
                .get("characters_written")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        ),
        crate::voice_tools::TERMINAL_KEY_TOOL => {
            let key = response
                .get("key")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("-");
            format!("Tasto inviato al terminale: {key}.")
        }
        crate::voice_tools::TERMINAL_CONFIGURE_TOOL => {
            "Configurazione terminale applicata e pannello riavviato.".to_string()
        }
        crate::voice_tools::MARKDOWN_READ_TOOL => format!(
            "Documento letto: {} righe.",
            response
                .get("total_lines")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        ),
        crate::voice_tools::MARKDOWN_SEARCH_TOOL => format!(
            "Ricerca completata: {} occorrenze.",
            response
                .get("match_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        ),
        crate::voice_tools::MARKDOWN_REPLACE_TOOL => format!(
            "Sostituzione completata: {} occorrenze.",
            response
                .get("replacement_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        ),
        crate::voice_tools::MARKDOWN_DELETE_LINE_TOOL => format!(
            "Riga {} rimossa.",
            response
                .get("deleted_line")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        ),
        _ => "Operazione completata.".to_string(),
    }
}

fn find_next_marker(lower: &str, from: usize) -> Option<(usize, &'static Marker)> {
    MARKERS
        .iter()
        .filter_map(|marker| {
            lower[from..]
                .find(marker.token)
                .map(|offset| (from + offset, marker))
        })
        .min_by_key(|(idx, marker)| (*idx, marker.token.len()))
}

fn parse_key(payload: &str) -> Option<VoiceKey> {
    let normalized = normalize(payload);
    if normalized.contains("control c")
        || normalized.contains("ctrl c")
        || normalized.contains("controllo c")
    {
        Some(VoiceKey::CtrlC)
    } else if normalized.contains("freccia giu") || normalized == "giu" || normalized == "down" {
        Some(VoiceKey::ArrowDown)
    } else if normalized.contains("freccia su") || normalized == "su" || normalized == "up" {
        Some(VoiceKey::ArrowUp)
    } else if normalized.contains("freccia sinistra")
        || normalized == "sinistra"
        || normalized == "left"
    {
        Some(VoiceKey::ArrowLeft)
    } else if normalized.contains("freccia destra")
        || normalized == "destra"
        || normalized == "right"
    {
        Some(VoiceKey::ArrowRight)
    } else if normalized.contains("invio")
        || normalized.contains("enter")
        || normalized.contains("return")
    {
        Some(VoiceKey::Enter)
    } else if normalized.contains("backspace") || normalized.contains("cancella indietro") {
        Some(VoiceKey::Backspace)
    } else if normalized == "canc" || normalized.contains("delete") {
        Some(VoiceKey::Delete)
    } else if normalized == "tab" || normalized.contains("tabulazione") {
        Some(VoiceKey::Tab)
    } else if normalized == "esc" || normalized.contains("escape") {
        Some(VoiceKey::Escape)
    } else {
        None
    }
}

fn parse_pax_command(payload: &str) -> PaxCommand {
    let normalized = normalize(payload);
    let select_tab_prefix = "seleziona tab ";
    if normalized.starts_with(select_tab_prefix) && payload.len() >= select_tab_prefix.len() {
        PaxCommand::SelectTab(payload[select_tab_prefix.len()..].trim().to_string())
    } else {
        PaxCommand::Raw(payload.to_string())
    }
}

fn key_bytes<'a>(panel_type: &str, key: &VoiceKey) -> Option<&'a [u8]> {
    let terminal = panel_type == "terminal";
    match key {
        VoiceKey::Enter if terminal => Some(b"\r"),
        VoiceKey::Enter => Some(b"\n"),
        VoiceKey::ArrowUp if terminal => Some(b"\x1b[A"),
        VoiceKey::ArrowDown if terminal => Some(b"\x1b[B"),
        VoiceKey::ArrowRight if terminal => Some(b"\x1b[C"),
        VoiceKey::ArrowLeft if terminal => Some(b"\x1b[D"),
        VoiceKey::Backspace => Some(&[0x7f]),
        VoiceKey::Delete if terminal => Some(b"\x1b[3~"),
        VoiceKey::Tab => Some(b"\t"),
        VoiceKey::Escape if terminal => Some(&[0x1b]),
        VoiceKey::CtrlC if terminal => Some(&[0x03]),
        _ => None,
    }
}

fn plan_preview(plan: &VoicePlan) -> String {
    if plan.actions.is_empty() && plan.issues.is_empty() {
        return "Nessuna azione riconosciuta.".to_string();
    }
    let mut parts: Vec<String> = plan.actions.iter().map(VoiceAction::label).collect();
    parts.extend(plan.issues.iter().map(|issue| format!("Avviso: {issue}")));
    parts.join(" · ")
}

fn execution_preview(plan: &VoicePlan, report: &VoiceExecutionReport) -> String {
    let mut text = format!(
        "{} azione/i eseguite. Piano: {}",
        report.executed,
        plan_preview(plan)
    );
    if !report.skipped.is_empty() {
        text.push_str("\nNon eseguito: ");
        text.push_str(&report.skipped.join(" · "));
    }
    text
}

fn strip_wrapping_quotes(text: &str) -> &str {
    let trimmed = text.trim();
    for (open, close) in [('"', '"'), ('\'', '\''), ('«', '»'), ('“', '”')] {
        if trimmed.starts_with(open) && trimmed.ends_with(close) {
            return trimmed
                .trim_start_matches(open)
                .trim_end_matches(close)
                .trim();
        }
    }
    trimmed
}

fn normalize(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .replace(['ù', 'ú'], "u")
        .replace(['ì', 'í'], "i")
        .replace(['è', 'é'], "e")
        .replace(['ò', 'ó'], "o")
        .replace(['à', 'á'], "a")
}

impl VoiceAction {
    fn label(&self) -> String {
        match self {
            VoiceAction::InsertText(text) => format!("Scrivi \"{}\"", text),
            VoiceAction::Newline => "Nuova riga".to_string(),
            VoiceAction::Key(key) => format!("Tastiera {}", key.label()),
            VoiceAction::Pax(command) => format!("Pax {}", command.label()),
        }
    }
}

impl VoiceKey {
    fn label(&self) -> &'static str {
        match self {
            VoiceKey::Enter => "Invio",
            VoiceKey::ArrowUp => "Freccia su",
            VoiceKey::ArrowDown => "Freccia giu",
            VoiceKey::ArrowLeft => "Freccia sinistra",
            VoiceKey::ArrowRight => "Freccia destra",
            VoiceKey::Backspace => "Backspace",
            VoiceKey::Delete => "Delete",
            VoiceKey::Tab => "Tab",
            VoiceKey::Escape => "Esc",
            VoiceKey::CtrlC => "Ctrl+C",
        }
    }
}

impl PaxCommand {
    fn label(&self) -> String {
        match self {
            PaxCommand::SelectTab(name) => format!("seleziona tab \"{}\"", name),
            PaxCommand::Raw(command) => command.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    use serial_test::serial;

    #[test]
    fn parses_write_newline_and_key_sequence() {
        let plan = parse_voice_phrase(
            "scrivi: ieri sono andato al mare va a capo scrivi: poi tastiera: invio",
        );
        assert_eq!(
            plan.actions,
            vec![
                VoiceAction::InsertText("ieri sono andato al mare".to_string()),
                VoiceAction::Newline,
                VoiceAction::InsertText("poi".to_string()),
                VoiceAction::Key(VoiceKey::Enter),
            ]
        );
        assert!(plan.issues.is_empty());
    }

    #[test]
    fn literal_write_keeps_command_words_as_text() {
        let plan = parse_voice_phrase("scrivi letteralmente: pax seleziona tab terminale");
        assert_eq!(
            plan.actions,
            vec![VoiceAction::InsertText(
                "pax seleziona tab terminale".to_string()
            )]
        );
    }

    #[test]
    fn parses_keyboard_arrows_and_ctrl_c() {
        assert_eq!(
            parse_voice_phrase("tastiera: freccia giù").actions,
            vec![VoiceAction::Key(VoiceKey::ArrowDown)]
        );
        assert_eq!(
            parse_voice_phrase("tastiera: control c").actions,
            vec![VoiceAction::Key(VoiceKey::CtrlC)]
        );
    }

    #[test]
    fn terminal_newline_requires_explicit_enter_key() {
        let out = RefCell::new(Vec::new());
        let plan = parse_voice_phrase("scrivi: ls va a capo tastiera: invio");
        let report = execute_voice_actions("terminal", &plan.actions, &|bytes| {
            out.borrow_mut().extend_from_slice(bytes);
            true
        });
        assert_eq!(out.into_inner(), b"ls\r");
        assert_eq!(report.executed, 2);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn markdown_newline_is_inserted() {
        let out = RefCell::new(Vec::new());
        let plan = parse_voice_phrase("scrivi: ciao va a capo scrivi: mondo");
        let report = execute_voice_actions("markdown", &plan.actions, &|bytes| {
            out.borrow_mut().extend_from_slice(bytes);
            true
        });
        assert_eq!(out.into_inner(), b"ciao\nmondo");
        assert_eq!(report.executed, 3);
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn tool_call_event_is_executed_and_answered() {
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        let call = crate::voice_tools::VoiceToolCall {
            id: "call-1".to_string(),
            name: crate::voice_tools::MARKDOWN_SEARCH_TOOL.to_string(),
            arguments: serde_json::json!({ "query": "xxx" }),
        };
        event_tx
            .send(crate::voice_session::VoiceSessionEvent::ToolCall {
                call,
                response: response_tx,
            })
            .unwrap();

        let done = drain_transcribe_events(
            &event_rx,
            &|_| {},
            &|_| {},
            &|_| {},
            &|_| {},
            &|_| {},
            &|call| {
                crate::voice_tools::VoiceToolExecution::immediate(
                    crate::voice_tools::VoiceToolResult {
                        call_id: call.id,
                        name: call.name,
                        response: serde_json::json!({ "status": "ok", "match_count": 1 }),
                    },
                )
            },
            &|| {},
        );

        assert!(done.is_none());
        let response = response_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let crate::voice_tools::VoiceToolExecution::Immediate(response) = response else {
            panic!("expected immediate execution");
        };
        assert_eq!(response.call_id, "call-1");
        assert_eq!(response.response["match_count"], 1);
    }

    #[test]
    fn assistant_transcript_event_reaches_the_ui_callback() {
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        event_tx
            .send(
                crate::voice_session::VoiceSessionEvent::AssistantTranscript(
                    "Sono l'assistente di Pax.".to_string(),
                ),
            )
            .unwrap();
        event_tx
            .send(crate::voice_session::VoiceSessionEvent::TurnComplete)
            .unwrap();
        let received = RefCell::new(String::new());
        let completed = Cell::new(false);

        let done = drain_transcribe_events(
            &event_rx,
            &|_| {},
            &|_| {},
            &|_| {},
            &|text| *received.borrow_mut() = text,
            &|_| {},
            &|call| {
                crate::voice_tools::VoiceToolExecution::immediate(
                    crate::voice_tools::VoiceToolResult::error(&call, "unexpected"),
                )
            },
            &|| completed.set(true),
        );

        assert!(done.is_none());
        assert_eq!(received.into_inner(), "Sono l'assistente di Pax.");
        assert!(completed.get());
    }

    #[test]
    #[serial]
    fn live_conversation_message_is_created_and_updated_without_reborrow() {
        crate::test_support::run_on_gtk_thread(|| {
            let chat_list = gtk4::ListBox::new();
            let chat_scroll = gtk4::ScrolledWindow::new();
            chat_scroll.set_child(Some(&chat_list));
            let history_list = gtk4::ListBox::new();
            let chat_empty = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            let history_empty = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            let feed = ConversationFeed::new(
                &chat_list,
                &chat_scroll,
                &history_list,
                &chat_empty,
                &history_empty,
                None,
            );

            feed.update_live(ConversationRole::Assistant, "Prima parte", "voice");
            feed.update_live(
                ConversationRole::Assistant,
                "Prima parte e seconda parte",
                "voice",
            );

            let live = feed.live_assistant.borrow();
            let message = live.as_ref().expect("assistant live message");
            assert_eq!(message.text, "Prima parte e seconda parte");
            assert_eq!(message.label.text(), "Prima parte e seconda parte");
            assert!(!chat_empty.is_visible());
        });
    }

    #[test]
    fn panic_payloads_are_rendered_for_session_errors() {
        assert_eq!(panic_payload_message(&"boom"), "boom");
        assert_eq!(
            panic_payload_message(&"owned boom".to_string()),
            "owned boom"
        );
    }

    #[test]
    #[ignore = "uses the configured Gemini key and the live service"]
    #[serial]
    fn configured_text_input_updates_the_conversation_ui() {
        crate::test_support::run_on_gtk_thread(|| {
            let parent = gtk4::Window::new();
            let window = build_voice_assistant_window(
                &parent,
                Rc::new(|| Some("markdown".to_string())),
                Rc::new(|_| true),
                Rc::new(|call| {
                    crate::voice_tools::VoiceToolExecution::immediate(
                        crate::voice_tools::VoiceToolResult::error(&call, "unexpected tool call"),
                    )
                }),
                None,
                None,
                None,
            );
            let entry = widget_tree_find_css(&window, "assistant-message-entry")
                .and_then(|widget| widget.downcast::<gtk4::Entry>().ok())
                .expect("assistant message entry");
            let mute = widget_tree_find_css(&window, "assistant-mute-button")
                .and_then(|widget| widget.downcast::<gtk4::ToggleButton>().ok())
                .expect("assistant mute button");
            mute.set_active(true);
            entry.set_text("Chi sei? Rispondi in una frase breve senza usare tool.");
            entry.emit_activate();

            let context = gtk4::glib::MainContext::default();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            let mut received = false;
            while std::time::Instant::now() < deadline && !received {
                while context.pending() {
                    context.iteration(false);
                }
                received = widget_tree_has_css(&window, "assistant-message-ai");
                std::thread::sleep(Duration::from_millis(16));
            }

            window.close();
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                received,
                "Gemini transcript did not reach the conversation UI"
            );
        });
    }

    #[test]
    #[serial]
    fn assistant_is_a_persistent_non_modal_window() {
        crate::test_support::run_on_gtk_thread(|| {
            let parent = gtk4::Window::new();
            let window = build_voice_assistant_window(
                &parent,
                Rc::new(|| Some("markdown".to_string())),
                Rc::new(|_| true),
                Rc::new(|call| {
                    crate::voice_tools::VoiceToolExecution::immediate(
                        crate::voice_tools::VoiceToolResult::error(&call, "not executed"),
                    )
                }),
                None,
                None,
                None,
            );

            assert!(!window.is_modal());
            assert!(window.hides_on_close());
            assert!(window.transient_for().is_some());
            assert!(window.has_css_class("voice-assistant-window"));
            assert!(widget_tree_has_css(&window, "assistant-view-switcher"));
            assert!(widget_tree_has_css(&window, "assistant-composer"));
            assert!(widget_tree_has_css(&window, "assistant-message-entry"));
            assert!(widget_tree_has_css(&window, "assistant-mute-button"));
            assert!(widget_tree_has_css(&window, "assistant-voice-picker"));
            assert!(widget_tree_has_css(&window, "assistant-chat-empty"));
            assert!(widget_tree_has_css(&window, "assistant-history-empty"));
            assert!(widget_tree_has_css(&window, "assistant-voice-strip"));
            if let Some(seconds) = std::env::var("PAX_ASSISTANT_VISUAL_TEST")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
            {
                window.present();
                let context = gtk4::glib::MainContext::default();
                let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
                while std::time::Instant::now() < deadline {
                    while context.pending() {
                        context.iteration(false);
                    }
                    std::thread::sleep(Duration::from_millis(16));
                }
            }
            window.close();
        });
    }

    #[test]
    fn assistant_panel_context_has_readable_labels() {
        assert_eq!(
            assistant_panel_context(Some("markdown".to_string())),
            "Focus: Editor Markdown"
        );
        assert_eq!(
            assistant_panel_context(Some("terminal".to_string())),
            "Focus: Terminale"
        );
        assert_eq!(assistant_panel_context(None), "Nessun pannello in focus");
    }

    fn widget_tree_has_css<W: IsA<gtk4::Widget>>(root: &W, class_name: &str) -> bool {
        widget_tree_find_css(root, class_name).is_some()
    }

    fn widget_tree_find_css<W: IsA<gtk4::Widget>>(
        root: &W,
        class_name: &str,
    ) -> Option<gtk4::Widget> {
        fn find(widget: &gtk4::Widget, class_name: &str) -> Option<gtk4::Widget> {
            if widget.has_css_class(class_name) {
                return Some(widget.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(found) = find(&widget, class_name) {
                    return Some(found);
                }
                child = widget.next_sibling();
            }
            None
        }

        find(root.as_ref(), class_name)
    }
}
