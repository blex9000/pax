use std::collections::HashMap;

use super::PanelBackend;

/// Metadata for a registered panel type.
#[derive(Clone)]
pub struct PanelTypeInfo {
    /// Unique identifier (e.g., "terminal", "markdown", "browser")
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
    /// Type-specific config (e.g., file path for markdown, URL for browser)
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
pub struct FnFactory<F: Fn(&PanelCreateConfig) -> Box<dyn PanelBackend> + Clone + Send + Sync + 'static>(pub F);

impl<F: Fn(&PanelCreateConfig) -> Box<dyn PanelBackend> + Clone + Send + Sync + 'static> CloneablePanelFactory for FnFactory<F> {
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
    pub fn create(&self, type_id: &str, config: &PanelCreateConfig) -> Option<Box<dyn PanelBackend>> {
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

/// Build the default registry with all built-in panel types.
pub fn build_default_registry() -> PanelRegistry {
    let mut reg = PanelRegistry::new();

    // Terminal
    reg.register(
        "terminal",
        "Terminal",
        "Local shell terminal",
        "utilities-terminal-symbolic",
        true,
        |config| {
            let shell = if config.shell.is_empty() { "/bin/bash" } else { &config.shell };
            let ws_dir = config.extra.get("__workspace_dir__").map(|s| s.as_str());
            let panel = super::terminal::TerminalPanel::new(
                shell,
                config.cwd.as_deref(),
                &config.env,
                ws_dir,
            );
            if let Some(cmds_str) = config.extra.get("__startup_commands__") {
                let cmds: Vec<String> = cmds_str.lines().map(|l| l.to_string()).collect();
                panel.send_commands(&cmds);
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
            let file = config.extra.get("file").map(|s| s.as_str()).unwrap_or("README.md");
            Box::new(super::markdown::MarkdownPanel::new(file))
        },
    );

    // Browser (placeholder)
    reg.register(
        "browser",
        "Web Browser",
        "Embedded web browser for dashboards",
        "web-browser-symbolic",
        false,
        |config| {
            let url = config.extra.get("url").map(|s| s.as_str()).unwrap_or("about:blank");
            let content = format!("# Browser\n\nURL: {}\n\n(WebKitGTK integration pending)", url);
            let tmp = std::env::temp_dir().join(format!("myterms_browser_{}.md", std::process::id()));
            std::fs::write(&tmp, &content).ok();
            Box::new(super::markdown::MarkdownPanel::new(
                tmp.to_str().unwrap_or("/tmp/placeholder.md"),
            ))
        },
    );

    // SSH Terminal
    reg.register(
        "ssh",
        "SSH Terminal",
        "Connect to remote host via SSH",
        "network-server-symbolic",
        true,
        |config| {
            let shell = if config.shell.is_empty() { "/bin/bash" } else { &config.shell };
            let ws_dir = config.extra.get("__workspace_dir__").map(|s| s.as_str());
            let terminal = super::terminal::TerminalPanel::new(
                shell,
                config.cwd.as_deref(),
                &config.env,
                ws_dir,
            );
            if let Some(host) = config.extra.get("host") {
                let user = config.extra.get("user");
                let password = config.extra.get("password");
                let ssh_target = if let Some(u) = user {
                    format!("{}@{}", u, host)
                } else {
                    host.clone()
                };
                let cmd = if let Some(pw) = password {
                    // Use sshpass to pass password non-interactively
                    format!("sshpass -p '{}' ssh -o StrictHostKeyChecking=accept-new {}", pw.replace('\'', "'\\''"), ssh_target)
                } else {
                    format!("ssh {}", ssh_target)
                };
                terminal.send_commands(&[cmd]);
            }
            Box::new(terminal)
        },
    );

    // Remote Tmux
    reg.register(
        "remote_tmux",
        "Remote Tmux",
        "Attach to remote tmux session via SSH",
        "network-workgroup-symbolic",
        true,
        |config| {
            let shell = if config.shell.is_empty() { "/bin/bash" } else { &config.shell };
            let ws_dir = config.extra.get("__workspace_dir__").map(|s| s.as_str());
            let terminal = super::terminal::TerminalPanel::new(
                shell,
                config.cwd.as_deref(),
                &config.env,
                ws_dir,
            );
            if let Some(host) = config.extra.get("host") {
                let session = config.extra.get("session").map(|s| s.as_str()).unwrap_or("main");
                let user = config.extra.get("user");
                let target = if let Some(u) = user {
                    format!("{}@{}", u, host)
                } else {
                    host.clone()
                };
                let cmd = format!("ssh -t {} 'tmux new-session -A -s {}'", target, session);
                terminal.send_commands(&[cmd]);
            }
            Box::new(terminal)
        },
    );

    reg
}
