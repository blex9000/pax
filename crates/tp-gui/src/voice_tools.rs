use regex::{NoExpand, RegexBuilder};
use serde_json::Value;

pub(crate) const WORKSPACE_SELECT_TAB_TOOL: &str = "workspace_select_tab";
pub(crate) const WORKSPACE_INSPECT_TOOL: &str = "workspace_inspect";
pub(crate) const WORKSPACE_ACTION_TOOL: &str = "workspace_action";
pub(crate) const TERMINAL_READ_TOOL: &str = "terminal_read";
pub(crate) const TERMINAL_WRITE_TOOL: &str = "terminal_write";
pub(crate) const TERMINAL_KEY_TOOL: &str = "terminal_key";
pub(crate) const TERMINAL_WAIT_TOOL: &str = "terminal_wait";
pub(crate) const TERMINAL_CONFIGURE_TOOL: &str = "terminal_configure";
pub(crate) const TASK_STATUS_TOOL: &str = "task_status";
pub(crate) const TASK_CANCEL_TOOL: &str = "task_cancel";
pub(crate) const MARKDOWN_READ_TOOL: &str = "markdown_read";
pub(crate) const MARKDOWN_SEARCH_TOOL: &str = "markdown_search";
pub(crate) const MARKDOWN_REPLACE_TOOL: &str = "markdown_replace";
pub(crate) const MARKDOWN_DELETE_LINE_TOOL: &str = "markdown_delete_line";

const DEFAULT_READ_LINES: usize = 100;
const MAX_READ_LINES: usize = 200;
const MAX_SEARCH_RESULTS: usize = 100;
const DEFAULT_TERMINAL_READ_LINES: usize = 60;
const MAX_TERMINAL_READ_LINES: usize = 200;
const MAX_TERMINAL_RESPONSE_CHARS: usize = 48_000;
const MAX_TERMINAL_LINE_CHARS: usize = 2_000;
const MAX_TERMINAL_WRITE_CHARS: usize = 16_384;
const MAX_TERMINAL_KEY_REPEAT: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VoiceToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VoiceToolResult {
    pub call_id: String,
    pub name: String,
    pub response: Value,
}

impl VoiceToolResult {
    pub(crate) fn error(call: &VoiceToolCall, message: impl Into<String>) -> Self {
        Self {
            call_id: call.id.clone(),
            name: call.name.clone(),
            response: serde_json::json!({
                "status": "error",
                "error": message.into()
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct VoiceToolCompletion {
    pub result: VoiceToolResult,
    pub delivery_ack: tokio::sync::oneshot::Sender<()>,
}

#[derive(Debug)]
pub(crate) enum VoiceToolExecution {
    Immediate(VoiceToolResult),
    Pending {
        task_id: String,
        receiver: tokio::sync::oneshot::Receiver<VoiceToolCompletion>,
    },
}

impl VoiceToolExecution {
    pub(crate) fn immediate(result: VoiceToolResult) -> Self {
        Self::Immediate(result)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MarkdownToolOutcome {
    pub response: Value,
    pub replacement_text: Option<String>,
}

pub(crate) fn is_markdown_tool(name: &str) -> bool {
    matches!(
        name,
        MARKDOWN_READ_TOOL
            | MARKDOWN_SEARCH_TOOL
            | MARKDOWN_REPLACE_TOOL
            | MARKDOWN_DELETE_LINE_TOOL
    )
}

pub(crate) fn is_terminal_tool(name: &str) -> bool {
    matches!(
        name,
        TERMINAL_READ_TOOL
            | TERMINAL_WRITE_TOOL
            | TERMINAL_KEY_TOOL
            | TERMINAL_WAIT_TOOL
            | TERMINAL_CONFIGURE_TOOL
    )
}

pub(crate) fn is_task_tool(name: &str) -> bool {
    matches!(name, TASK_STATUS_TOOL | TASK_CANCEL_TOOL)
}

pub(crate) fn is_assistant_tool(name: &str) -> bool {
    matches!(
        name,
        WORKSPACE_SELECT_TAB_TOOL | WORKSPACE_INSPECT_TOOL | WORKSPACE_ACTION_TOOL
    ) || is_terminal_tool(name)
        || is_markdown_tool(name)
        || is_task_tool(name)
}

pub(crate) fn assistant_tool_declarations() -> Vec<Value> {
    let mut declarations = vec![
        serde_json::json!({
            "name": WORKSPACE_INSPECT_TOOL,
            "description": "Return the current complete Pax workspace structure. The result includes every recursive tab group, every tab's panel_ids and panel_count, panel names/types/visibility/focus/collapse/input-sync state, and current tab selections. Use it whenever the user asks about tabs, panels, counts, layout, visibility, or focus and the current structure is not certain.",
            "parameters": {
                "type": "OBJECT",
                "properties": {}
            }
        }),
        serde_json::json!({
            "name": WORKSPACE_SELECT_TAB_TOOL,
            "description": "Select a visible tab in the current Pax workspace by label. Use this for requests to show, open, switch to, or select a workspace tab.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "tab_name": {
                        "type": "STRING",
                        "description": "The tab label spoken by the user. Pax accepts exact labels or one unambiguous partial match."
                    }
                },
                "required": ["tab_name"]
            }
        }),
        serde_json::json!({
            "name": WORKSPACE_ACTION_TOOL,
            "description": "Perform a structural or configuration action through the same model-first workspace operations used by the Pax GUI. Use panel IDs and tab paths from workspace_inspect. The result always contains a fresh workspace snapshot. Destructive actions require confirm=true only after the user explicitly confirms.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "action": {
                        "type": "STRING",
                        "enum": [
                            "focus_panel",
                            "split_horizontal",
                            "split_vertical",
                            "add_tab",
                            "add_tab_to_group",
                            "insert_before",
                            "insert_after",
                            "move_left",
                            "move_right",
                            "move_up",
                            "move_down",
                            "toggle_zoom",
                            "toggle_sync_input",
                            "expand_panel",
                            "close_panel",
                            "close_tab",
                            "rename_panel",
                            "rename_tab",
                            "set_panel_type",
                            "reset_panel",
                            "rename_workspace",
                            "save_workspace"
                        ]
                    },
                    "panel_id": {
                        "type": "STRING",
                        "description": "Target panel ID. Omit only when the focused panel is the intended target."
                    },
                    "tab_name": {
                        "type": "STRING",
                        "description": "Visible tab label, required by close_tab."
                    },
                    "tab_path": {
                        "type": "ARRAY",
                        "items": { "type": "INTEGER" },
                        "description": "Exact recursive tab group path from workspace_inspect, required by add_tab_to_group."
                    },
                    "name": {
                        "type": "STRING",
                        "description": "New name for rename_panel, rename_tab, or rename_workspace."
                    },
                    "panel_type": {
                        "type": "STRING",
                        "enum": ["terminal", "markdown", "code_editor", "docker_help", "note"]
                    },
                    "confirm": {
                        "type": "BOOLEAN",
                        "description": "Must be true for close_panel, close_tab, or reset_panel after explicit user confirmation."
                    }
                },
                "required": ["action"]
            }
        }),
    ];
    declarations.extend(terminal_tool_declarations());
    declarations.extend(task_tool_declarations());
    declarations.extend(markdown_tool_declarations());
    declarations
}

pub(crate) fn workspace_inspection(snapshot: &pax_assistant::WorkspaceSnapshot) -> Value {
    let mut inspection = snapshot.provider_context();
    let mut tab_groups = Vec::new();
    collect_tab_groups(&snapshot.layout, &mut Vec::new(), &mut tab_groups);
    if let Some(object) = inspection.as_object_mut() {
        object.insert("tab_groups".to_string(), Value::Array(tab_groups));
    }
    inspection
}

pub(crate) fn execute_workspace_tool(
    view: &mut crate::workspace_view::WorkspaceView,
    call: &VoiceToolCall,
) -> Option<VoiceToolResult> {
    let result = match call.name.as_str() {
        WORKSPACE_INSPECT_TOOL => Ok(serde_json::json!({ "inspected": true })),
        WORKSPACE_SELECT_TAB_TOOL => {
            let tab_name = required_string(&call.arguments, "tab_name");
            tab_name.and_then(|tab_name| {
                view.focus_tab_by_label(&tab_name)
                    .map(|selected_tab| serde_json::json!({ "selected_tab": selected_tab }))
            })
        }
        WORKSPACE_ACTION_TOOL => execute_workspace_action(view, &call.arguments),
        _ => return None,
    };

    let snapshot = workspace_inspection(&view.assistant_snapshot());
    Some(match result {
        Ok(detail) => VoiceToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            response: serde_json::json!({
                "status": detail
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("ok"),
                "result": detail,
                "workspace": snapshot
            }),
        },
        Err(error) => VoiceToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            response: serde_json::json!({
                "status": "error",
                "error": error,
                "workspace": snapshot
            }),
        },
    })
}

fn execute_workspace_action(
    view: &mut crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<Value, String> {
    let action = required_string(arguments, "action")?;
    let detail = match action.as_str() {
        "focus_panel" => {
            let panel_id = focus_action_target(view, arguments)?;
            serde_json::json!({ "panel_id": panel_id })
        }
        "split_horizontal" => {
            let source_panel_id = focus_action_target(view, arguments)?;
            let panel_id = view
                .split_focused_h()
                .ok_or_else(|| "Impossibile creare lo split orizzontale.".to_string())?;
            serde_json::json!({
                "source_panel_id": source_panel_id,
                "created_panel_id": panel_id
            })
        }
        "split_vertical" => {
            let source_panel_id = focus_action_target(view, arguments)?;
            let panel_id = view
                .split_focused_v()
                .ok_or_else(|| "Impossibile creare lo split verticale.".to_string())?;
            serde_json::json!({
                "source_panel_id": source_panel_id,
                "created_panel_id": panel_id
            })
        }
        "add_tab" => {
            let source_panel_id = focus_action_target(view, arguments)?;
            let panel_id = view
                .add_tab_focused()
                .ok_or_else(|| "Impossibile aggiungere il tab.".to_string())?;
            serde_json::json!({
                "source_panel_id": source_panel_id,
                "created_panel_id": panel_id
            })
        }
        "add_tab_to_group" => {
            let tab_path = tab_path_arg(arguments)?;
            let panel_id = view
                .add_tab_to_tabs_path(&tab_path)
                .ok_or_else(|| format!("Nessun gruppo tab trovato al path {tab_path:?}."))?;
            serde_json::json!({
                "tab_path": tab_path,
                "created_panel_id": panel_id
            })
        }
        "insert_before" | "insert_after" => {
            let source_panel_id = focus_action_target(view, arguments)?;
            let position = if action == "insert_before" {
                crate::layout_ops::InsertPosition::Before
            } else {
                crate::layout_ops::InsertPosition::After
            };
            let panel_id = view
                .insert_sibling_focused(position)
                .ok_or_else(|| format!("Impossibile eseguire {action}."))?;
            serde_json::json!({
                "source_panel_id": source_panel_id,
                "created_panel_id": panel_id
            })
        }
        "move_left" | "move_right" | "move_up" | "move_down" => {
            let panel_id = focus_action_target(view, arguments)?;
            let direction = match action.as_str() {
                "move_left" => crate::workspace_view::MoveDirection::Left,
                "move_right" => crate::workspace_view::MoveDirection::Right,
                "move_up" => crate::workspace_view::MoveDirection::Up,
                "move_down" => crate::workspace_view::MoveDirection::Down,
                _ => unreachable!(),
            };
            if !view.move_focused_panel(direction) {
                return Err(format!(
                    "Il pannello '{panel_id}' non puo' essere spostato con {action}."
                ));
            }
            serde_json::json!({ "panel_id": panel_id })
        }
        "toggle_zoom" => {
            let panel_id = focus_action_target(view, arguments)?;
            view.toggle_zoom();
            serde_json::json!({
                "panel_id": panel_id,
                "zoomed": view.is_zoomed()
            })
        }
        "toggle_sync_input" => {
            let panel_id = focus_action_target(view, arguments)?;
            let (_, enabled) = view.toggle_sync_focused().ok_or_else(|| {
                format!("Il pannello '{panel_id}' non supporta la sincronizzazione input.")
            })?;
            serde_json::json!({
                "panel_id": panel_id,
                "enabled": enabled,
                "synced_panel_count": view.sync_count()
            })
        }
        "expand_panel" => {
            let panel_id = action_target_panel_id(view, arguments)?;
            let host = view
                .host(&panel_id)
                .ok_or_else(|| format!("Pannello '{panel_id}' non disponibile."))?;
            let was_collapsed = host.is_collapsed();
            host.expand_collapsed();
            serde_json::json!({
                "panel_id": panel_id,
                "was_collapsed": was_collapsed,
                "collapsed": host.is_collapsed()
            })
        }
        "close_panel" => {
            require_destructive_confirmation(arguments, &action)?;
            let panel_id = focus_action_target(view, arguments)?;
            if !view.close_focused() {
                return Err("Impossibile chiudere l'ultimo pannello del workspace.".to_string());
            }
            serde_json::json!({ "closed_panel_id": panel_id })
        }
        "close_tab" => {
            require_destructive_confirmation(arguments, &action)?;
            let tab_name = required_string(arguments, "tab_name")?;
            let (closed_tab, closed_panel_ids) = view.close_tab_by_label(&tab_name)?;
            serde_json::json!({
                "closed_tab": closed_tab,
                "closed_panel_ids": closed_panel_ids
            })
        }
        "rename_panel" => {
            let panel_id = action_target_panel_id(view, arguments)?;
            let name = required_string(arguments, "name")?;
            if !view.rename_panel(&panel_id, &name) {
                return Err(format!(
                    "Pannello '{panel_id}' non trovato o nome invariato."
                ));
            }
            serde_json::json!({ "panel_id": panel_id, "name": name })
        }
        "rename_tab" => {
            let panel_id = action_target_panel_id(view, arguments)?;
            let name = required_string(arguments, "name")?;
            if !view.rename_tab_label(&panel_id, &name) {
                return Err(format!("Nessun tab contiene il pannello '{panel_id}'."));
            }
            view.refresh_tab_labels();
            serde_json::json!({ "panel_id": panel_id, "name": name })
        }
        "set_panel_type" => {
            let panel_id = action_target_panel_id(view, arguments)?;
            let panel_type = required_string(arguments, "panel_type")?;
            if !matches!(
                panel_type.as_str(),
                "terminal" | "markdown" | "code_editor" | "docker_help" | "note"
            ) {
                return Err(format!("Tipo pannello non supportato: {panel_type}."));
            }
            let needs_configuration = view.set_panel_type(&panel_id, &panel_type);
            serde_json::json!({
                "panel_id": panel_id,
                "panel_type": panel_type,
                "needs_configuration": needs_configuration
            })
        }
        "reset_panel" => {
            require_destructive_confirmation(arguments, &action)?;
            let panel_id = action_target_panel_id(view, arguments)?;
            view.reset_panel(&panel_id);
            serde_json::json!({ "panel_id": panel_id })
        }
        "rename_workspace" => {
            let name = required_string(arguments, "name")?;
            view.rename_workspace(&name);
            serde_json::json!({ "name": name })
        }
        "save_workspace" => {
            let path = view.save()?;
            serde_json::json!({ "path": path })
        }
        _ => return Err(format!("Azione workspace non supportata: {action}.")),
    };

    Ok(serde_json::json!({
        "status": "ok",
        "action": action,
        "detail": detail
    }))
}

fn action_target_panel_id(
    view: &crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<String, String> {
    let panel_id = arguments
        .get("panel_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| view.focused_panel_id().map(str::to_string))
        .ok_or_else(|| "Nessun pannello target o in focus.".to_string())?;
    if view.workspace().panel(&panel_id).is_none() {
        return Err(format!("Pannello '{panel_id}' non trovato."));
    }
    Ok(panel_id)
}

fn focus_action_target(
    view: &mut crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<String, String> {
    let panel_id = action_target_panel_id(view, arguments)?;
    if !view.focus_panel(&panel_id) {
        return Err(format!("Impossibile focalizzare il pannello '{panel_id}'."));
    }
    Ok(panel_id)
}

fn tab_path_arg(arguments: &Value) -> Result<Vec<usize>, String> {
    arguments
        .get("tab_path")
        .and_then(Value::as_array)
        .ok_or_else(|| "tab_path deve essere un array di indici.".to_string())?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| "tab_path contiene un indice non valido.".to_string())
        })
        .collect()
}

fn require_destructive_confirmation(arguments: &Value, action: &str) -> Result<(), String> {
    if bool_arg(arguments, "confirm", false) {
        Ok(())
    } else {
        Err(format!(
            "L'azione distruttiva '{action}' richiede conferma esplicita."
        ))
    }
}

fn collect_tab_groups(
    node: &pax_assistant::LayoutSnapshot,
    path: &mut Vec<usize>,
    groups: &mut Vec<Value>,
) {
    use pax_assistant::LayoutSnapshot;

    match node {
        LayoutSnapshot::Panel { .. } => {}
        LayoutSnapshot::HorizontalSplit { children, .. }
        | LayoutSnapshot::VerticalSplit { children, .. } => {
            for (index, child) in children.iter().enumerate() {
                path.push(index);
                collect_tab_groups(child, path, groups);
                path.pop();
            }
        }
        LayoutSnapshot::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            let tabs = children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    let mut panel_ids = Vec::new();
                    collect_panel_ids(child, &mut panel_ids);
                    serde_json::json!({
                        "index": index,
                        "label": labels.get(index),
                        "tab_id": tab_ids.get(index),
                        "panel_count": panel_ids.len(),
                        "panel_ids": panel_ids
                    })
                })
                .collect::<Vec<_>>();
            groups.push(serde_json::json!({
                "path": path,
                "tabs": tabs
            }));
            for (index, child) in children.iter().enumerate() {
                path.push(index);
                collect_tab_groups(child, path, groups);
                path.pop();
            }
        }
    }
}

fn collect_panel_ids(node: &pax_assistant::LayoutSnapshot, panel_ids: &mut Vec<String>) {
    use pax_assistant::LayoutSnapshot;

    match node {
        LayoutSnapshot::Panel { panel_id } => panel_ids.push(panel_id.clone()),
        LayoutSnapshot::HorizontalSplit { children, .. }
        | LayoutSnapshot::VerticalSplit { children, .. }
        | LayoutSnapshot::Tabs { children, .. } => {
            for child in children {
                collect_panel_ids(child, panel_ids);
            }
        }
    }
}

pub(crate) fn terminal_tool_declarations() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": TERMINAL_READ_TOOL,
            "description": "Read only a bounded window of recent output from one terminal panel. Terminal output is not part of the persistent workspace context: call this only when the user's request requires inspecting a terminal or checking the effect of a prior terminal input.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "panel_id": {
                        "type": "STRING",
                        "description": "Terminal panel ID from workspace_inspect. Omit only when the focused panel is the intended terminal."
                    },
                    "last_lines": {
                        "type": "INTEGER",
                        "description": "Number of recent lines to return. Defaults to 60 and is capped at 200."
                    }
                }
            }
        }),
        serde_json::json!({
            "name": TERMINAL_WRITE_TOOL,
            "description": "Type printable text into one terminal or interactive terminal application. This never presses Enter and does not execute or submit the text. Use terminal_key separately for Enter, navigation, selection, or control keys.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "panel_id": {
                        "type": "STRING",
                        "description": "Terminal panel ID from workspace_inspect. Omit only when the focused panel is the intended terminal."
                    },
                    "text": {
                        "type": "STRING",
                        "description": "Exact printable text to type, up to 16384 characters. Control characters and newlines are rejected."
                    }
                },
                "required": ["text"]
            }
        }),
        serde_json::json!({
            "name": TERMINAL_KEY_TOOL,
            "description": "Send a real terminal key to a shell or interactive TUI such as Codex or Claude Code. The result includes output_revision and, for Enter at a shell prompt, watch_token. Pass those values to terminal_wait when the requested operation needs follow-through. Read the terminal first when the current choice or prompt is uncertain. Use Enter only when the user explicitly asked to execute, submit, confirm, or choose the current option.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "panel_id": {
                        "type": "STRING",
                        "description": "Terminal panel ID from workspace_inspect. Omit only when the focused panel is the intended terminal."
                    },
                    "key": {
                        "type": "STRING",
                        "enum": [
                            "enter",
                            "tab",
                            "shift_tab",
                            "escape",
                            "up",
                            "down",
                            "left",
                            "right",
                            "home",
                            "end",
                            "page_up",
                            "page_down",
                            "backspace",
                            "delete",
                            "insert",
                            "space",
                            "ctrl_a",
                            "ctrl_c",
                            "ctrl_d",
                            "ctrl_e",
                            "ctrl_l",
                            "ctrl_n",
                            "ctrl_p",
                            "ctrl_r",
                            "ctrl_u",
                            "ctrl_w",
                            "ctrl_z"
                        ]
                    },
                    "repeat": {
                        "type": "INTEGER",
                        "description": "Number of consecutive key presses, defaults to 1 and is limited to 20."
                    }
                },
                "required": ["key"]
            }
        }),
        serde_json::json!({
            "name": TERMINAL_WAIT_TOOL,
            "description": "Wait asynchronously for a meaningful terminal condition, then return only bounded recent output. Use this after starting a command or interacting with a TUI when the user expects Pax to follow the operation through without another user message. Prefer shell_prompt for normal shell commands, output_quiet for interactive screens, output_changed for the next update, or contains_text for a known prompt. This tool remains pending until the condition, timeout, or cancellation.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "panel_id": {
                        "type": "STRING",
                        "description": "Terminal panel ID. Omit only when the focused panel is the intended terminal."
                    },
                    "condition": {
                        "type": "STRING",
                        "enum": ["shell_prompt", "output_changed", "output_quiet", "contains_text"]
                    },
                    "watch_token": {
                        "type": "INTEGER",
                        "description": "Command generation returned by terminal_key when condition is shell_prompt."
                    },
                    "after_revision": {
                        "type": "INTEGER",
                        "description": "Output revision returned by terminal_key or terminal_read. Output conditions only consider changes after this revision."
                    },
                    "quiet_ms": {
                        "type": "INTEGER",
                        "description": "Required stable period for output_quiet, from 250 to 10000 ms. Defaults to 900."
                    },
                    "text": {
                        "type": "STRING",
                        "description": "Text to detect for contains_text."
                    },
                    "case_sensitive": {
                        "type": "BOOLEAN",
                        "description": "Whether contains_text is case-sensitive. Defaults to false."
                    },
                    "timeout_seconds": {
                        "type": "INTEGER",
                        "description": "Maximum wait from 1 to 600 seconds. Defaults to 300."
                    },
                    "output_lines": {
                        "type": "INTEGER",
                        "description": "Recent lines returned on completion, from 1 to 200. Defaults to 60."
                    },
                    "label": {
                        "type": "STRING",
                        "description": "Short activity label shown in the Pax pending-task UI."
                    }
                },
                "required": ["condition"]
            }
        }),
        serde_json::json!({
            "name": TERMINAL_CONFIGURE_TOOL,
            "description": "Update a terminal panel configuration through the same apply path as the Pax GUI. Applying configuration restarts that terminal backend, so confirm=true is required only after the user explicitly confirms the restart. Omitted fields are preserved.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "panel_id": {
                        "type": "STRING",
                        "description": "Terminal panel ID from workspace_inspect. Omit only when the focused panel is the intended terminal."
                    },
                    "name": {
                        "type": "STRING",
                        "description": "New panel name."
                    },
                    "cwd": {
                        "type": "STRING",
                        "description": "Working directory. An empty string clears the configured directory."
                    },
                    "startup_commands": {
                        "type": "ARRAY",
                        "items": { "type": "STRING" },
                        "description": "Complete replacement list of startup commands. An empty array disables startup commands."
                    },
                    "before_close": {
                        "type": "STRING",
                        "description": "Script to run before closing/restarting. An empty string clears it."
                    },
                    "min_width": {
                        "type": "INTEGER",
                        "description": "Minimum width in pixels; 0 disables the minimum."
                    },
                    "min_height": {
                        "type": "INTEGER",
                        "description": "Minimum height in pixels; 0 disables the minimum."
                    },
                    "ssh_enabled": {
                        "type": "BOOLEAN",
                        "description": "Enable or disable the terminal's already configured SSH target."
                    },
                    "confirm": {
                        "type": "BOOLEAN",
                        "description": "Must be true after explicit confirmation that applying configuration restarts this terminal."
                    }
                },
                "required": ["confirm"]
            }
        }),
    ]
}

pub(crate) fn task_tool_declarations() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": TASK_STATUS_TOOL,
            "description": "Inspect one pending or recently completed Pax assistant task by ID. Omit task_id to list current tasks without terminal output.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "task_id": {
                        "type": "STRING"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": TASK_CANCEL_TOOL,
            "description": "Cancel a pending Pax assistant task. This stops monitoring only; it does not terminate the underlying terminal process. Use terminal_key ctrl_c separately only when the user explicitly asks to interrupt that process.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "task_id": {
                        "type": "STRING"
                    }
                },
                "required": ["task_id"]
            }
        }),
    ]
}

pub(crate) fn execute_terminal_tool(
    view: &mut crate::workspace_view::WorkspaceView,
    call: &VoiceToolCall,
) -> Option<VoiceToolResult> {
    let result = match call.name.as_str() {
        TERMINAL_READ_TOOL => execute_terminal_read(view, &call.arguments),
        TERMINAL_WRITE_TOOL => execute_terminal_write(view, &call.arguments),
        TERMINAL_KEY_TOOL => execute_terminal_key(view, &call.arguments),
        TERMINAL_CONFIGURE_TOOL => execute_terminal_configure(view, &call.arguments),
        _ => return None,
    };

    Some(match result {
        Ok(response) => VoiceToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            response,
        },
        Err(error) => VoiceToolResult::error(call, error),
    })
}

fn execute_terminal_configure(
    view: &mut crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<Value, String> {
    require_destructive_confirmation(arguments, TERMINAL_CONFIGURE_TOOL)?;
    let panel_id = terminal_panel_id(view, arguments)?;
    let current = view
        .workspace()
        .panel(&panel_id)
        .cloned()
        .ok_or_else(|| format!("Pannello '{panel_id}' non trovato."))?;

    let name = optional_non_empty_string(arguments, "name")?.unwrap_or(current.name.clone());
    let cwd = optional_clearable_string(arguments, "cwd")?.unwrap_or(current.cwd.clone());
    let startup_commands = optional_string_array(arguments, "startup_commands")?
        .unwrap_or_else(|| current.startup_commands.clone());
    let before_close = optional_clearable_string(arguments, "before_close")?
        .unwrap_or_else(|| current.before_close.clone());
    let min_width = optional_u32(arguments, "min_width")?.unwrap_or(current.min_width);
    let min_height = optional_u32(arguments, "min_height")?.unwrap_or(current.min_height);
    let ssh_enabled = arguments
        .get("ssh_enabled")
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| "ssh_enabled deve essere booleano.".to_string())
        })
        .transpose()?
        .unwrap_or_else(|| current.effective_ssh_enabled());

    let update = pax_core::workspace::PanelConfigUpdate {
        name,
        panel_type: pax_core::workspace::PanelType::Terminal,
        cwd,
        ssh: current.effective_ssh(),
        ssh_enabled,
        startup_commands,
        before_close,
        min_width,
        min_height,
    };
    let changed = update.name != current.name
        || update.cwd != current.cwd
        || update.ssh_enabled != current.effective_ssh_enabled()
        || update.startup_commands != current.startup_commands
        || update.before_close != current.before_close
        || update.min_width != current.min_width
        || update.min_height != current.min_height;
    if !changed {
        return Err("La configurazione richiesta e' identica a quella corrente.".to_string());
    }

    view.apply_panel_config(&panel_id, update.clone());
    Ok(serde_json::json!({
        "status": "ok",
        "panel_id": panel_id,
        "restarted": true,
        "configuration": {
            "name": update.name,
            "cwd": update.cwd,
            "startup_command_count": update.startup_commands.len(),
            "before_close_configured": update.before_close.is_some(),
            "min_width": update.min_width,
            "min_height": update.min_height,
            "ssh_configured": update.ssh.is_some(),
            "ssh_enabled": update.ssh_enabled
        }
    }))
}

fn execute_terminal_read(
    view: &crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<Value, String> {
    let (panel_id, host) = terminal_target(view, arguments)?;
    let requested_lines = positive_usize(arguments, "last_lines")?
        .unwrap_or(DEFAULT_TERMINAL_READ_LINES)
        .min(MAX_TERMINAL_READ_LINES);
    let content = host
        .text_content()
        .ok_or_else(|| format!("Impossibile leggere l'output del terminale '{panel_id}'."))?;
    let mut response = recent_terminal_output(&panel_id, &content, requested_lines);
    let runtime = host.terminal_runtime_snapshot();
    if let Some(object) = response.as_object_mut() {
        object.insert(
            "output_revision".to_string(),
            Value::from(runtime.output_revision),
        );
        object.insert("busy".to_string(), Value::from(runtime.busy));
        object.insert(
            "command_generation".to_string(),
            Value::from(runtime.command_generation),
        );
        object.insert(
            "completed_generation".to_string(),
            Value::from(runtime.completed_generation),
        );
    }
    Ok(response)
}

fn execute_terminal_write(
    view: &crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<Value, String> {
    let text = required_string(arguments, "text")?;
    if text.chars().any(char::is_control) {
        return Err(
            "terminal_write accetta solo testo stampabile; usa terminal_key per Invio, Tab e altri tasti."
                .to_string(),
        );
    }
    let character_count = text.chars().count();
    if character_count > MAX_TERMINAL_WRITE_CHARS {
        return Err(format!(
            "terminal_write non puo' superare {MAX_TERMINAL_WRITE_CHARS} caratteri."
        ));
    }
    let (panel_id, host) = terminal_target(view, arguments)?;
    if !host.write_input(text.as_bytes()) {
        return Err(format!(
            "Il terminale '{panel_id}' non ha accettato il testo."
        ));
    }
    Ok(serde_json::json!({
        "status": "ok",
        "panel_id": panel_id,
        "bytes_written": text.len(),
        "characters_written": character_count,
        "submitted": false
    }))
}

fn execute_terminal_key(
    view: &crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<Value, String> {
    let key = required_string(arguments, "key")?;
    let key_bytes = crate::panels::terminal::encode_named_key_input(&key)
        .ok_or_else(|| format!("Tasto terminale non supportato: {key}."))?;
    let repeat = positive_usize(arguments, "repeat")?.unwrap_or(1);
    if repeat > MAX_TERMINAL_KEY_REPEAT {
        return Err(format!(
            "repeat non puo' superare {MAX_TERMINAL_KEY_REPEAT}."
        ));
    }
    let (panel_id, host) = terminal_target(view, arguments)?;
    let runtime = host.terminal_runtime_snapshot();
    let watch_token = (key == "enter").then(|| {
        if runtime.busy {
            runtime.command_generation
        } else {
            runtime.command_generation.wrapping_add(1)
        }
    });
    let mut input = Vec::with_capacity(key_bytes.len() * repeat);
    for _ in 0..repeat {
        input.extend_from_slice(key_bytes);
    }
    if !host.write_input(&input) {
        return Err(format!(
            "Il terminale '{panel_id}' non ha accettato il tasto."
        ));
    }
    Ok(serde_json::json!({
        "status": "ok",
        "panel_id": panel_id,
        "key": key,
        "repeat": repeat,
        "after_revision": runtime.output_revision,
        "watch_token": watch_token
    }))
}

fn terminal_target<'a>(
    view: &'a crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<(String, &'a crate::panel_host::PanelHost), String> {
    let panel_id = terminal_panel_id(view, arguments)?;
    let host = view
        .host(&panel_id)
        .ok_or_else(|| format!("Terminale '{panel_id}' non disponibile."))?;
    Ok((panel_id, host))
}

pub(crate) fn terminal_panel_id(
    view: &crate::workspace_view::WorkspaceView,
    arguments: &Value,
) -> Result<String, String> {
    let panel_id = arguments
        .get("panel_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| view.focused_panel_id().map(str::to_string))
        .ok_or_else(|| "Nessun terminale target o in focus.".to_string())?;
    let panel = view
        .workspace()
        .panel(&panel_id)
        .ok_or_else(|| format!("Pannello '{panel_id}' non trovato."))?;
    if !matches!(
        panel.effective_type(),
        pax_core::workspace::PanelType::Terminal
    ) {
        return Err(format!("Il pannello '{panel_id}' non e' un terminale."));
    }
    Ok(panel_id)
}

pub(crate) fn recent_terminal_output(
    panel_id: &str,
    content: &str,
    requested_lines: usize,
) -> Value {
    let lines = content.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let requested_start = total_lines.saturating_sub(requested_lines);
    let mut remaining_chars = MAX_TERMINAL_RESPONSE_CHARS;
    let mut selected = Vec::new();

    for index in (requested_start..total_lines).rev() {
        let (text, line_truncated) = truncate_chars(lines[index], MAX_TERMINAL_LINE_CHARS);
        let cost = text.chars().count().saturating_add(1);
        if !selected.is_empty() && cost > remaining_chars {
            break;
        }
        remaining_chars = remaining_chars.saturating_sub(cost);
        selected.push(serde_json::json!({
            "number": index + 1,
            "text": text,
            "truncated": line_truncated
        }));
        if remaining_chars == 0 {
            break;
        }
    }
    selected.reverse();
    let start_line = selected
        .first()
        .and_then(|line| line.get("number"))
        .and_then(Value::as_u64);

    serde_json::json!({
        "status": "ok",
        "panel_id": panel_id,
        "total_lines": total_lines,
        "start_line": start_line,
        "returned_lines": selected.len(),
        "lines": selected,
        "has_earlier_output": start_line.is_some_and(|line| line > 1),
        "bounded": true
    })
}

fn truncate_chars(value: &str, limit: usize) -> (String, bool) {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(limit).collect::<String>();
    let has_more = chars.next().is_some();
    (truncated, has_more)
}

pub(crate) fn markdown_tool_declarations() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": MARKDOWN_READ_TOOL,
            "description": "Read the focused Markdown document as numbered lines. Use this before edits that depend on existing content.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "start_line": { "type": "INTEGER", "description": "First 1-based line, defaults to 1." },
                    "end_line": { "type": "INTEGER", "description": "Last 1-based line, inclusive. At most 200 lines are returned." }
                }
            }
        }),
        serde_json::json!({
            "name": MARKDOWN_SEARCH_TOOL,
            "description": "Find text in the focused Markdown document and return matching line and column positions.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "query": { "type": "STRING" },
                    "case_sensitive": { "type": "BOOLEAN", "description": "Defaults to false." }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": MARKDOWN_REPLACE_TOOL,
            "description": "Replace exact text in the focused Markdown document. Returns the number of replacements.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "query": { "type": "STRING" },
                    "replacement": { "type": "STRING" },
                    "replace_all": { "type": "BOOLEAN", "description": "Defaults to true." },
                    "case_sensitive": { "type": "BOOLEAN", "description": "Defaults to false." }
                },
                "required": ["query", "replacement"]
            }
        }),
        serde_json::json!({
            "name": MARKDOWN_DELETE_LINE_TOOL,
            "description": "Delete one line from the focused Markdown document by 1-based line number or by position.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "line": { "type": "INTEGER", "description": "Specific 1-based line number." },
                    "position": {
                        "type": "STRING",
                        "enum": ["last", "last_nonempty"],
                        "description": "Use last_nonempty when the request means the last line containing text."
                    }
                }
            }
        }),
    ]
}

pub(crate) fn execute_markdown_tool(
    call: &VoiceToolCall,
    content: &str,
) -> Result<MarkdownToolOutcome, String> {
    match call.name.as_str() {
        MARKDOWN_READ_TOOL => read_lines(&call.arguments, content),
        MARKDOWN_SEARCH_TOOL => search_text(&call.arguments, content),
        MARKDOWN_REPLACE_TOOL => replace_text(&call.arguments, content),
        MARKDOWN_DELETE_LINE_TOOL => delete_line(&call.arguments, content),
        _ => Err(format!("Unsupported Markdown tool: {}", call.name)),
    }
}

trait PanelToolAdapter {
    fn handles(&self, tool_name: &str) -> bool;

    fn execute(
        &self,
        view: &crate::workspace_view::WorkspaceView,
        call: &VoiceToolCall,
    ) -> VoiceToolResult;
}

struct MarkdownPanelToolAdapter;

impl PanelToolAdapter for MarkdownPanelToolAdapter {
    fn handles(&self, tool_name: &str) -> bool {
        is_markdown_tool(tool_name)
    }

    fn execute(
        &self,
        view: &crate::workspace_view::WorkspaceView,
        call: &VoiceToolCall,
    ) -> VoiceToolResult {
        let outcome = (|| {
            let panel_id = view
                .focused_panel_id()
                .ok_or_else(|| "Nessun pannello ha il focus.".to_string())?;
            let panel = view
                .workspace()
                .panel(panel_id)
                .ok_or_else(|| format!("Pannello '{panel_id}' non trovato."))?;
            if !matches!(
                panel.effective_type(),
                pax_core::workspace::PanelType::Markdown { .. }
            ) {
                return Err("Il pannello focalizzato non e' un documento Markdown.".to_string());
            }
            let host = view
                .host(panel_id)
                .ok_or_else(|| "Pannello Markdown non disponibile.".to_string())?;
            let content = host
                .text_content()
                .ok_or_else(|| "Impossibile leggere il documento Markdown.".to_string())?;
            let outcome = execute_markdown_tool(call, &content)?;
            if let Some(updated) = outcome.replacement_text.as_deref() {
                if !host.replace_text_content(updated) {
                    return Err("Impossibile aggiornare il documento Markdown.".to_string());
                }
            }
            Ok(outcome.response)
        })();

        match outcome {
            Ok(response) => VoiceToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                response,
            },
            Err(error) => VoiceToolResult::error(call, error),
        }
    }
}

pub(crate) fn execute_panel_tool(
    view: &crate::workspace_view::WorkspaceView,
    call: &VoiceToolCall,
) -> Option<VoiceToolResult> {
    let adapters: [&dyn PanelToolAdapter; 1] = [&MarkdownPanelToolAdapter];
    adapters
        .into_iter()
        .find(|adapter| adapter.handles(&call.name))
        .map(|adapter| adapter.execute(view, call))
}

fn read_lines(arguments: &Value, content: &str) -> Result<MarkdownToolOutcome, String> {
    let lines = document_lines(content);
    let start = positive_usize(arguments, "start_line")?.unwrap_or(1);
    let requested_end = positive_usize(arguments, "end_line")?;
    let end = requested_end
        .unwrap_or_else(|| start.saturating_add(DEFAULT_READ_LINES - 1))
        .min(start.saturating_add(MAX_READ_LINES - 1))
        .min(lines.len());
    if start > lines.len() && !lines.is_empty() {
        return Err(format!(
            "start_line {start} is outside the document ({} lines)",
            lines.len()
        ));
    }
    let selected = if lines.is_empty() || start > end {
        Vec::new()
    } else {
        lines[start - 1..end]
            .iter()
            .enumerate()
            .map(|(offset, line)| serde_json::json!({ "number": start + offset, "text": line }))
            .collect::<Vec<_>>()
    };
    Ok(MarkdownToolOutcome {
        response: serde_json::json!({
            "status": "ok",
            "total_lines": lines.len(),
            "lines": selected,
            "has_more": end < lines.len()
        }),
        replacement_text: None,
    })
}

fn search_text(arguments: &Value, content: &str) -> Result<MarkdownToolOutcome, String> {
    let query = required_string(arguments, "query")?;
    let regex = literal_regex(&query, bool_arg(arguments, "case_sensitive", false))?;
    let mut matches = Vec::new();
    let mut total_matches = 0usize;
    for (line_index, line) in document_lines(content).iter().enumerate() {
        for found in regex.find_iter(line) {
            total_matches += 1;
            if matches.len() < MAX_SEARCH_RESULTS {
                matches.push(serde_json::json!({
                    "line": line_index + 1,
                    "column": line[..found.start()].chars().count() + 1,
                    "text": line
                }));
            }
        }
    }
    Ok(MarkdownToolOutcome {
        response: serde_json::json!({
            "status": "ok",
            "query": query,
            "match_count": total_matches,
            "matches": matches,
            "truncated": total_matches > MAX_SEARCH_RESULTS
        }),
        replacement_text: None,
    })
}

fn replace_text(arguments: &Value, content: &str) -> Result<MarkdownToolOutcome, String> {
    let query = required_string(arguments, "query")?;
    let replacement = arguments
        .get("replacement")
        .and_then(Value::as_str)
        .ok_or_else(|| "replacement must be a string".to_string())?;
    let regex = literal_regex(&query, bool_arg(arguments, "case_sensitive", false))?;
    let replace_all = bool_arg(arguments, "replace_all", true);
    let replacement_count = if replace_all {
        regex.find_iter(content).count()
    } else {
        usize::from(regex.find(content).is_some())
    };
    let updated = if replace_all {
        regex
            .replace_all(content, NoExpand(replacement))
            .into_owned()
    } else {
        regex.replace(content, NoExpand(replacement)).into_owned()
    };
    Ok(MarkdownToolOutcome {
        response: serde_json::json!({
            "status": if replacement_count == 0 { "not_found" } else { "ok" },
            "query": query,
            "replacement_count": replacement_count,
            "total_lines": document_lines(&updated).len()
        }),
        replacement_text: (replacement_count > 0).then_some(updated),
    })
}

fn delete_line(arguments: &Value, content: &str) -> Result<MarkdownToolOutcome, String> {
    let spans = line_spans(content);
    if spans.is_empty() {
        return Err("The Markdown document is empty".to_string());
    }
    let line_number = if let Some(line) = positive_usize(arguments, "line")? {
        line
    } else {
        match arguments.get("position").and_then(Value::as_str) {
            Some("last") => spans.len(),
            Some("last_nonempty") => spans
                .iter()
                .rposition(|span| !span.text.trim().is_empty())
                .map(|index| index + 1)
                .ok_or_else(|| "The Markdown document has no non-empty lines".to_string())?,
            Some(position) => return Err(format!("Unsupported line position: {position}")),
            None => return Err("Provide line or position".to_string()),
        }
    };
    let Some(span) = spans.get(line_number - 1) else {
        return Err(format!(
            "line {line_number} is outside the document ({} lines)",
            spans.len()
        ));
    };
    let mut updated = content.to_string();
    updated.replace_range(span.start..span.end, "");
    Ok(MarkdownToolOutcome {
        response: serde_json::json!({
            "status": "ok",
            "deleted_line": line_number,
            "deleted_text": span.text,
            "total_lines": line_spans(&updated).len()
        }),
        replacement_text: Some(updated),
    })
}

fn literal_regex(query: &str, case_sensitive: bool) -> Result<regex::Regex, String> {
    if query.is_empty() {
        return Err("query cannot be empty".to_string());
    }
    RegexBuilder::new(&regex::escape(query))
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| error.to_string())
}

fn required_string(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{name} must be a non-empty string"))
}

fn positive_usize(arguments: &Value, name: &str) -> Result<Option<usize>, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    let value = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{name} must be a positive integer"))?;
    Ok(Some(value))
}

fn bool_arg(arguments: &Value, name: &str, default: bool) -> bool {
    arguments
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn optional_non_empty_string(arguments: &Value, name: &str) -> Result<Option<String>, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| format!("{name} deve essere una stringa non vuota"))
}

fn optional_clearable_string(
    arguments: &Value,
    name: &str,
) -> Result<Option<Option<String>>, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| format!("{name} deve essere una stringa"))?;
    Ok(Some((!value.trim().is_empty()).then(|| value.to_string())))
}

fn optional_string_array(arguments: &Value, name: &str) -> Result<Option<Vec<String>>, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{name} deve essere un array di stringhe"))?;
    if values.len() > 100 {
        return Err(format!("{name} non puo' contenere piu' di 100 elementi"));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{name} contiene un valore non testuale"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn optional_u32(arguments: &Value, name: &str) -> Result<Option<u32>, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value <= 10_000)
        .map(Some)
        .ok_or_else(|| format!("{name} deve essere compreso tra 0 e 10000"))
}

fn document_lines(content: &str) -> Vec<&str> {
    content.lines().collect()
}

struct LineSpan<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

fn line_spans(content: &str) -> Vec<LineSpan<'_>> {
    let mut offset = 0usize;
    content
        .split_inclusive('\n')
        .map(|segment| {
            let start = offset;
            offset += segment.len();
            LineSpan {
                start,
                end: offset,
                text: segment.trim_end_matches(['\r', '\n']),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pax_assistant::{LayoutSnapshot, WorkspaceSnapshot, WORKSPACE_SNAPSHOT_VERSION};
    use uuid::Uuid;

    fn call(name: &str, arguments: Value) -> VoiceToolCall {
        VoiceToolCall {
            id: "call-1".into(),
            name: name.into(),
            arguments,
        }
    }

    #[test]
    fn workspace_inspection_counts_panels_inside_each_tab() {
        let snapshot = WorkspaceSnapshot {
            version: WORKSPACE_SNAPSHOT_VERSION,
            workspace_id: Uuid::new_v4(),
            record_key: "workspace".into(),
            name: "Workspace".into(),
            config_path: None,
            dirty: false,
            focused_panel_id: Some("p2".into()),
            zoomed_panel_id: None,
            active_tabs: Vec::new(),
            layout: LayoutSnapshot::Tabs {
                children: vec![
                    LayoutSnapshot::HorizontalSplit {
                        children: vec![
                            LayoutSnapshot::Panel {
                                panel_id: "p1".into(),
                            },
                            LayoutSnapshot::Panel {
                                panel_id: "p2".into(),
                            },
                        ],
                        ratios: vec![0.5, 0.5],
                    },
                    LayoutSnapshot::Panel {
                        panel_id: "p3".into(),
                    },
                ],
                labels: vec!["FREEFLOW".into(), "QUALITY-GURU_DOCVAL".into()],
                tab_ids: vec!["tab-freeflow".into(), "tab-quality".into()],
            },
            panels: Vec::new(),
        };

        let inspection = workspace_inspection(&snapshot);

        assert_eq!(
            inspection.pointer("/tab_groups/0/tabs/0/panel_count"),
            Some(&serde_json::json!(2))
        );
        assert_eq!(
            inspection.pointer("/tab_groups/0/tabs/0/panel_ids"),
            Some(&serde_json::json!(["p1", "p2"]))
        );
        assert_eq!(
            inspection.pointer("/tab_groups/0/tabs/1/panel_count"),
            Some(&serde_json::json!(1))
        );
    }

    #[test]
    fn workspace_tools_expose_inspection_before_mutation_tools() {
        let declarations = assistant_tool_declarations();
        assert_eq!(
            declarations[0].get("name").and_then(Value::as_str),
            Some(WORKSPACE_INSPECT_TOOL)
        );
        assert_eq!(
            declarations[3].get("name").and_then(Value::as_str),
            Some(TERMINAL_READ_TOOL)
        );
        assert_eq!(
            declarations[5].get("name").and_then(Value::as_str),
            Some(TERMINAL_KEY_TOOL)
        );
        assert_eq!(
            declarations[6].get("name").and_then(Value::as_str),
            Some(TERMINAL_WAIT_TOOL)
        );
        assert_eq!(
            declarations[8].get("name").and_then(Value::as_str),
            Some(TASK_STATUS_TOOL)
        );
        assert_eq!(declarations.len(), 14);
        assert!(is_assistant_tool(WORKSPACE_INSPECT_TOOL));
        assert!(is_assistant_tool(TERMINAL_READ_TOOL));
        assert!(is_assistant_tool(TERMINAL_WAIT_TOOL));
    }

    #[test]
    fn terminal_read_returns_only_the_requested_recent_lines() {
        let content = (1..=100)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        let response = recent_terminal_output("term-1", &content, 3);

        assert_eq!(response["total_lines"], 100);
        assert_eq!(response["returned_lines"], 3);
        assert_eq!(response["lines"][0]["number"], 98);
        assert_eq!(response["lines"][0]["text"], "line 98");
        assert_eq!(response["lines"][2]["text"], "line 100");
        assert_eq!(response["has_earlier_output"], true);
        assert_eq!(response["bounded"], true);
    }

    #[test]
    fn terminal_read_truncates_long_unicode_lines_at_character_boundaries() {
        let content = "è".repeat(MAX_TERMINAL_LINE_CHARS + 1);

        let response = recent_terminal_output("term-1", &content, 1);

        assert_eq!(
            response["lines"][0]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            MAX_TERMINAL_LINE_CHARS
        );
        assert_eq!(response["lines"][0]["truncated"], true);
    }

    #[test]
    fn reads_numbered_line_ranges() {
        let outcome = execute_markdown_tool(
            &call(MARKDOWN_READ_TOOL, serde_json::json!({"start_line": 2})),
            "one\ntwo\nthree",
        )
        .unwrap();

        assert_eq!(outcome.response["total_lines"], 3);
        assert_eq!(outcome.response["lines"][0]["number"], 2);
        assert_eq!(outcome.response["lines"][0]["text"], "two");
    }

    #[test]
    fn searches_case_insensitively_with_line_and_column() {
        let outcome = execute_markdown_tool(
            &call(MARKDOWN_SEARCH_TOOL, serde_json::json!({"query": "xxx"})),
            "hello XXX\nxxx again",
        )
        .unwrap();

        assert_eq!(outcome.response["match_count"], 2);
        assert_eq!(outcome.response["matches"][0]["line"], 1);
        assert_eq!(outcome.response["matches"][0]["column"], 7);
    }

    #[test]
    fn replaces_literal_text_without_expanding_dollar_signs() {
        let outcome = execute_markdown_tool(
            &call(
                MARKDOWN_REPLACE_TOOL,
                serde_json::json!({"query": "xxx", "replacement": "$yy"}),
            ),
            "xxx and XXX",
        )
        .unwrap();

        assert_eq!(outcome.response["replacement_count"], 2);
        assert_eq!(outcome.replacement_text.as_deref(), Some("$yy and $yy"));
    }

    #[test]
    fn deletes_last_nonempty_line_and_preserves_prior_newline() {
        let outcome = execute_markdown_tool(
            &call(
                MARKDOWN_DELETE_LINE_TOOL,
                serde_json::json!({"position": "last_nonempty"}),
            ),
            "one\ntwo\n\n",
        )
        .unwrap();

        assert_eq!(outcome.response["deleted_line"], 2);
        assert_eq!(outcome.replacement_text.as_deref(), Some("one\n\n"));
    }
}
