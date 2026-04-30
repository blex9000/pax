use std::collections::HashMap;

use pax_core::workspace::{PanelConfig, PanelType, SshConfig};

use crate::panels::markdown::MarkdownPanel;
use crate::panels::registry::{PanelCreateConfig, PanelRegistry};

pub fn panel_type_to_id(pt: &PanelType) -> &'static str {
    match pt {
        PanelType::Empty => "__empty__",
        PanelType::Terminal | PanelType::Ssh { .. } | PanelType::RemoteTmux { .. } => "terminal",
        PanelType::Markdown { .. } => "markdown",
        PanelType::CodeEditor { .. } => "code_editor",
        PanelType::Note => "note",
    }
}

pub fn panel_type_to_create_config(
    pt: &PanelType,
    default_shell: &str,
    workspace_dir: Option<&str>,
    workspace_record_key: Option<&str>,
) -> PanelCreateConfig {
    let mut extra = HashMap::new();
    match pt {
        PanelType::Markdown { file } => {
            extra.insert("file".to_string(), file.clone());
        }
        PanelType::CodeEditor {
            root_dir,
            ssh,
            remote_path,
            ..
        } => {
            extra.insert("root_dir".to_string(), root_dir.clone());
            if let Some(ref ssh_cfg) = ssh {
                extra.insert("ssh_host".to_string(), ssh_cfg.host.clone());
                if let Some(ref u) = ssh_cfg.user {
                    extra.insert("ssh_user".to_string(), u.clone());
                }
                if let Some(ref p) = ssh_cfg.password {
                    extra.insert("ssh_password".to_string(), p.clone());
                }
                if let Some(ref k) = ssh_cfg.identity_file {
                    extra.insert("ssh_identity".to_string(), k.clone());
                }
                extra.insert("ssh_port".to_string(), ssh_cfg.port.to_string());
            }
            if let Some(ref rp) = remote_path {
                extra.insert("remote_path".to_string(), rp.clone());
            }
        }
        _ => {}
    }
    if let Some(dir) = workspace_dir {
        extra.insert("__workspace_dir__".to_string(), dir.to_string());
    }
    if let Some(rk) = workspace_record_key {
        extra.insert("__workspace_record_key__".to_string(), rk.to_string());
    }
    PanelCreateConfig {
        shell: default_shell.to_string(),
        cwd: None,
        env: vec![],
        extra,
    }
}

pub fn insert_ssh_extra(extra: &mut HashMap<String, String>, ssh: &SshConfig) {
    extra.insert("ssh_host".to_string(), ssh.host.clone());
    if let Some(ref u) = ssh.user {
        extra.insert("ssh_user".to_string(), u.clone());
    }
    if let Some(ref p) = ssh.password {
        extra.insert("ssh_password".to_string(), p.clone());
    }
    if let Some(ref s) = ssh.tmux_session {
        extra.insert("ssh_tmux_session".to_string(), s.clone());
    }
}

pub fn create_backend_from_registry(
    panel_cfg: &PanelConfig,
    default_shell: &str,
    registry: &PanelRegistry,
    workspace_dir: Option<&str>,
    workspace_record_key: Option<&str>,
) -> Box<dyn crate::panels::PanelBackend> {
    let effective = panel_cfg.effective_type();
    let (type_id, mut extra) = match &effective {
        PanelType::Empty => ("__empty__", HashMap::new()),
        PanelType::Terminal | PanelType::Ssh { .. } | PanelType::RemoteTmux { .. } => {
            ("terminal", HashMap::new())
        }
        PanelType::Markdown { file } => {
            let mut extra = HashMap::new();
            extra.insert("file".to_string(), file.clone());
            ("markdown", extra)
        }
        PanelType::CodeEditor {
            root_dir,
            ssh,
            remote_path,
            ..
        } => {
            let mut extra = HashMap::new();
            extra.insert("root_dir".to_string(), root_dir.clone());
            if let Some(ref ssh_cfg) = ssh {
                extra.insert("ssh_host".to_string(), ssh_cfg.host.clone());
                if let Some(ref u) = ssh_cfg.user {
                    extra.insert("ssh_user".to_string(), u.clone());
                }
                if let Some(ref p) = ssh_cfg.password {
                    extra.insert("ssh_password".to_string(), p.clone());
                }
                if let Some(ref k) = ssh_cfg.identity_file {
                    extra.insert("ssh_identity".to_string(), k.clone());
                }
                extra.insert("ssh_port".to_string(), ssh_cfg.port.to_string());
            }
            if let Some(ref rp) = remote_path {
                extra.insert("remote_path".to_string(), rp.clone());
            }
            ("code_editor", extra)
        }
        PanelType::Note => {
            let mut extra = HashMap::new();
            extra.insert("__panel_id__".to_string(), panel_cfg.id.clone());
            ("note", extra)
        }
    };

    if let Some(ref ssh) = panel_cfg.effective_ssh() {
        insert_ssh_extra(&mut extra, ssh);
    }

    if !panel_cfg.startup_commands.is_empty() {
        extra.insert(
            "__startup_commands__".to_string(),
            panel_cfg.startup_commands.join("\n"),
        );
    }
    if let Some(dir) = workspace_dir {
        extra.insert("__workspace_dir__".to_string(), dir.to_string());
    }
    if let Some(rk) = workspace_record_key {
        extra.insert("__workspace_record_key__".to_string(), rk.to_string());
    }
    extra.insert(
        "__panel_uuid__".to_string(),
        panel_cfg.uuid.simple().to_string(),
    );
    let config = PanelCreateConfig {
        shell: default_shell.to_string(),
        cwd: panel_cfg.cwd.clone(),
        env: panel_cfg
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        extra,
    };

    registry
        .create(type_id, &config)
        .unwrap_or_else(|| Box::new(MarkdownPanel::new("/dev/null")))
}
