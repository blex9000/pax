use std::collections::HashMap;

use super::PanelBackend;

/// Metadata for a registered panel type.
#[derive(Clone)]
pub struct PanelTypeInfo {
    /// Unique identifier (e.g., "terminal", "markdown")
    pub id: String,
    /// Human-readable name shown in the chooser
    pub display_name: String,
    /// Description shown in the chooser
    pub description: String,
    /// GTK icon name
    pub icon: String,
    /// Whether this panel type accepts keyboard input
    pub accepts_input: bool,
    /// Factory function: creates a PanelBackend, optionally with config
    factory: Box<dyn CloneablePanelFactory>,
}

/// Config passed to the factory when creating a panel.
#[derive(Debug, Clone, Default)]
pub struct PanelCreateConfig {
    pub shell: String,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    /// Type-specific config (e.g., file path for markdown)
    pub extra: HashMap<String, String>,
}

/// Trait for panel factories that can be cloned (for storage in registry).
pub trait CloneablePanelFactory: Send + Sync {
    fn create(&self, config: &PanelCreateConfig) -> Box<dyn PanelBackend>;
    fn clone_box(&self) -> Box<dyn CloneablePanelFactory>;
}

impl Clone for Box<dyn CloneablePanelFactory> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Simple factory wrapper for a function pointer.
#[derive(Clone)]
pub struct FnFactory<
    F: Fn(&PanelCreateConfig) -> Box<dyn PanelBackend> + Clone + Send + Sync + 'static,
>(pub F);

impl<F: Fn(&PanelCreateConfig) -> Box<dyn PanelBackend> + Clone + Send + Sync + 'static>
    CloneablePanelFactory for FnFactory<F>
{
    fn create(&self, config: &PanelCreateConfig) -> Box<dyn PanelBackend> {
        (self.0)(config)
    }
    fn clone_box(&self) -> Box<dyn CloneablePanelFactory> {
        Box::new(self.clone())
    }
}

/// Dynamic registry of panel types. Panel types register themselves here.
/// New panel types can be added at runtime (plugin-style).
#[derive(Clone, Default)]
pub struct PanelRegistry {
    types: Vec<PanelTypeInfo>,
}

impl PanelRegistry {
    pub fn new() -> Self {
        Self { types: Vec::new() }
    }

    /// Register a new panel type.
    pub fn register<F>(
        &mut self,
        id: &str,
        display_name: &str,
        description: &str,
        icon: &str,
        accepts_input: bool,
        factory: F,
    ) where
        F: Fn(&PanelCreateConfig) -> Box<dyn PanelBackend> + Clone + Send + Sync + 'static,
    {
        self.types.push(PanelTypeInfo {
            id: id.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            icon: icon.to_string(),
            accepts_input,
            factory: Box::new(FnFactory(factory)),
        });
    }

    /// Get all registered panel types.
    pub fn types(&self) -> &[PanelTypeInfo] {
        &self.types
    }

    /// Create a panel by type ID.
    pub fn create(
        &self,
        type_id: &str,
        config: &PanelCreateConfig,
    ) -> Option<Box<dyn PanelBackend>> {
        self.types
            .iter()
            .find(|t| t.id == type_id)
            .map(|t| t.factory.create(config))
    }

    /// Get info for a type.
    pub fn get_type(&self, type_id: &str) -> Option<&PanelTypeInfo> {
        self.types.iter().find(|t| t.id == type_id)
    }
}

/// Default markdown file name used when the user doesn't specify one.
const DEFAULT_MARKDOWN_FILE_NAME: &str = "README.md";

/// Resolve `path` against `workspace_dir` when `path` is relative.
/// Returns `path` unchanged when it is absolute, or when `workspace_dir` is
/// `None` (e.g. an unsaved workspace), preserving the pre-existing behavior.
fn resolve_against_workspace(path: &str, workspace_dir: Option<&str>) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    match workspace_dir {
        Some(dir) => std::path::Path::new(dir)
            .join(path)
            .to_string_lossy()
            .to_string(),
        None => path.to_string(),
    }
}

/// Like `resolve_against_workspace`, but when no workspace directory is set
/// (unsaved workspace) it anchors relative paths to `$HOME` instead of leaving
/// them relative to the process cwd. Used for files we auto-create on behalf
/// of the user (markdown README) so the result is predictable regardless of
/// where the app binary was launched from.
fn resolve_path_predictable(path: &str, workspace_dir: Option<&str>) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    if let Some(dir) = workspace_dir {
        return std::path::Path::new(dir)
            .join(path)
            .to_string_lossy()
            .to_string();
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return std::path::Path::new(&home)
                .join(path)
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string()
}

/// Ensure `path` exists as a file. Creates parent directories and an empty
/// file if missing. Logs and swallows errors so the panel still opens (the
/// markdown viewer will surface the load failure to the user).
fn ensure_markdown_file_exists(path: &str) {
    let p = std::path::Path::new(path);
    if p.exists() {
        return;
    }
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    "ensure_markdown_file_exists: could not create parent {}: {}",
                    parent.display(),
                    e
                );
                return;
            }
        }
    }
    if let Err(e) = std::fs::write(p, b"") {
        tracing::warn!(
            "ensure_markdown_file_exists: could not create {}: {}",
            p.display(),
            e
        );
    }
}

/// Build the default registry with all built-in panel types.
pub fn build_default_registry() -> PanelRegistry {
    let mut reg = PanelRegistry::new();

    // Terminal
    reg.register(
        "terminal",
        "Terminal",
        "Local or remote shell terminal",
        "utilities-terminal-symbolic",
        true,
        |config| {
            let default_shell = std::env::var("SHELL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "/bin/bash".to_string());
            let shell = if config.shell.is_empty() {
                default_shell.as_str()
            } else {
                &config.shell
            };
            let ws_dir = config.extra.get("__workspace_dir__").map(|s| s.as_str());
            // Add SSHPASS to env if password configured (never visible in terminal)
            let mut env = config.env.clone();
            if let Some(pw) = config.extra.get("ssh_password") {
                env.push(("SSHPASS".to_string(), pw.clone()));
            }
            // Default cwd: $HOME if the user didn't specify one. We
            // deliberately do NOT fall through ws_dir here — that would make
            // the same PanelConfig open in different directories depending on
            // whether the workspace has been saved (ws_dir is only known for
            // saved workspaces). Anchoring the default to $HOME keeps shell
            // behavior identical across sessions and across saves. An explicit
            // empty string in cwd is treated as "unspecified".
            let home_owned = std::env::var("HOME")
                .ok()
                .filter(|s| !s.is_empty());
            let effective_cwd = config
                .cwd
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(home_owned.as_deref());
            let mut panel = super::terminal::TerminalPanel::new(
                shell,
                effective_cwd,
                &env,
                ws_dir,
            );
            let is_ssh = config.extra.contains_key("ssh_host");

            // SSH connection if configured
            if let Some(host) = config.extra.get("ssh_host") {
                let user = config.extra.get("ssh_user");
                // Set SSH label for panel header indicator
                let ssh_label = if let Some(u) = user {
                    format!("{}@{}", u, host)
                } else {
                    host.clone()
                };
                panel.set_ssh_info(ssh_label);
                let has_password = config.extra.contains_key("ssh_password");
                let tmux_session = config.extra.get("ssh_tmux_session");
                let ssh_target = if let Some(u) = user {
                    format!("{}@{}", u, host)
                } else {
                    host.clone()
                };
                let cmd = if let Some(session) = tmux_session {
                    if has_password {
                        format!("sshpass -e ssh -o StrictHostKeyChecking=accept-new -t {} 'tmux new-session -A -s {}'", ssh_target, session)
                    } else {
                        format!("ssh -t {} 'tmux new-session -A -s {}'", ssh_target, session)
                    }
                } else if has_password {
                    format!("sshpass -e ssh -o StrictHostKeyChecking=accept-new {}", ssh_target)
                } else {
                    format!("ssh {}", ssh_target)
                };
                panel.send_commands(&[cmd]);
                // cd to remote path if specified
                if let Some(cwd) = config.cwd.as_deref() {
                    if !cwd.is_empty() {
                        panel.send_commands(&[format!("cd '{}'", cwd)]);
                    }
                }
                // Override PS1 and set PROMPT_COMMAND for OSC 7 on the remote shell
                // (same as local terminal — minimal prompt + directory tracking +
                // colored ls). Intentionally no trailing `clear`: when SSH fails
                // the remote shell never executes these commands, and the fallback
                // `clear` would end up wiping the SSH error from the local shell,
                // turning connection failures into silent no-ops.
                panel.send_commands(&[
                    " export PS1='\\[\\033[32m\\]$:\\[\\033[0m\\] '".to_string(),
                    " export PROMPT_COMMAND='printf \"\\033]7;file://%s%s\\033\\\\\" \"$HOSTNAME\" \"$PWD\"'".to_string(),
                    " export LS_COLORS='di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42'".to_string(),
                ]);
            }
            // Startup script commands
            if let Some(cmds_str) = config.extra.get("__startup_commands__") {
                if is_ssh {
                    // For SSH: wrap script in a heredoc so it runs on the remote host.
                    // Strip shebang — the heredoc pipes to bash directly.
                    // Use queue_raw to avoid temp file processing.
                    let script_body: String = cmds_str.lines()
                        .filter(|l| !l.starts_with("#!"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !script_body.trim().is_empty() {
                        let heredoc = format!(
                            "cat << 'PAX_SCRIPT_EOF' | bash\n{}\nPAX_SCRIPT_EOF",
                            script_body
                        );
                        panel.queue_raw(&heredoc);
                    }
                } else {
                    let cmds: Vec<String> = cmds_str.lines().map(|l| l.to_string()).collect();
                    panel.send_commands(&cmds);
                }
            }
            Box::new(panel)
        },
    );

    // Markdown
    reg.register(
        "markdown",
        "Markdown Viewer",
        "View and render markdown files",
        "text-x-generic-symbolic",
        false,
        |config| {
            let ws_dir = config.extra.get("__workspace_dir__").map(|s| s.as_str());
            // Fall back to the default name when the user didn't specify a file,
            // then anchor relative names to the workspace directory so "notes.md"
            // means "inside this workspace".
            let raw_file = config
                .extra
                .get("file")
                .map(|s| s.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_MARKDOWN_FILE_NAME);
            // Use the predictable variant: for unsaved workspaces we prefer
            // $HOME/README.md over a path relative to the process cwd, so the
            // file doesn't silently land in the directory the binary was
            // launched from.
            let resolved = resolve_path_predictable(raw_file, ws_dir);
            // If the file doesn't exist yet, create it (empty) so the viewer
            // opens successfully instead of showing a load error.
            ensure_markdown_file_exists(&resolved);
            Box::new(super::markdown::MarkdownPanel::new(&resolved))
        },
    );

    // Code Editor
    reg.register(
        "code_editor",
        "Code Editor",
        "Lightweight code editor with file tree and git",
        "accessories-text-editor-symbolic",
        true,
        |config| {
            let ws_dir = config.extra.get("__workspace_dir__").map(|s| s.as_str());
            // Treat an unset, empty, or "." root_dir as "use the workspace dir"
            // — that's what the user means when they leave it blank.
            let raw_root = config
                .extra
                .get("root_dir")
                .map(|s| s.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != ".")
                .map(|s| s.to_string());
            let root_dir: String = match raw_root {
                Some(ref r) => resolve_against_workspace(r, ws_dir),
                None => ws_dir.map(|s| s.to_string()).unwrap_or_else(|| ".".to_string()),
            };
            let ssh_host = config.extra.get("ssh_host").cloned();
            let ssh_user = config.extra.get("ssh_user").cloned();
            let ssh_password = config.extra.get("ssh_password").cloned();
            let ssh_identity = config.extra.get("ssh_identity").cloned();
            let ssh_port = config
                .extra
                .get("ssh_port")
                .and_then(|s| s.parse().ok())
                .unwrap_or(22u16);
            let remote_path = config.extra.get("remote_path").cloned();
            let record_key = config
                .extra
                .get("__workspace_record_key__")
                .cloned()
                .unwrap_or_default();

            if let Some(host) = ssh_host {
                // Remote code editor: mount via SSHFS
                let user = ssh_user.as_deref().unwrap_or("root");
                let rpath = remote_path.as_deref().unwrap_or(root_dir.as_str());
                Box::new(super::editor::CodeEditorPanel::new_remote(
                    &host,
                    ssh_port,
                    user,
                    ssh_password.as_deref(),
                    ssh_identity.as_deref(),
                    rpath,
                    record_key,
                ))
            } else {
                Box::new(super::editor::CodeEditorPanel::new(&root_dir, record_key))
            }
        },
    );

    reg
}

#[cfg(test)]
mod tests {
    use super::build_default_registry;

    #[test]
    fn default_registry_does_not_expose_browser_panel() {
        let registry = build_default_registry();

        assert!(registry.get_type("browser").is_none());
        assert!(registry.types().iter().all(|panel| panel.id != "browser"));
    }
}
