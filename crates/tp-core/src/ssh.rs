use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed SSH host entry.
#[derive(Debug, Clone)]
pub struct SshHost {
    pub name: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub extra: HashMap<String, String>,
}

/// Parse ~/.ssh/config and return all host entries.
pub fn parse_ssh_config(path: &Path) -> Result<Vec<SshHost>> {
    let content = std::fs::read_to_string(path)?;
    parse_ssh_config_str(&content)
}

/// Parse from the default location ~/.ssh/config.
pub fn parse_default_ssh_config() -> Result<Vec<SshHost>> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/root"));
    let path = home.join(".ssh").join("config");
    if path.exists() {
        parse_ssh_config(&path)
    } else {
        Ok(Vec::new())
    }
}

fn parse_ssh_config_str(content: &str) -> Result<Vec<SshHost>> {
    let mut hosts = Vec::new();
    let mut current: Option<SshHost> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match line.split_once(char::is_whitespace) {
            Some((k, v)) => (k.to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        if key == "host" {
            // Skip wildcard-only patterns
            if value.contains('*') || value.contains('?') {
                if let Some(h) = current.take() {
                    hosts.push(h);
                }
                continue;
            }
            if let Some(h) = current.take() {
                hosts.push(h);
            }
            current = Some(SshHost {
                name: value,
                hostname: None,
                user: None,
                port: None,
                identity_file: None,
                proxy_jump: None,
                extra: HashMap::new(),
            });
        } else if let Some(ref mut h) = current {
            match key.as_str() {
                "hostname" => h.hostname = Some(value),
                "user" => h.user = Some(value),
                "port" => h.port = value.parse().ok(),
                "identityfile" => h.identity_file = Some(value),
                "proxyjump" => h.proxy_jump = Some(value),
                _ => {
                    h.extra.insert(key, value);
                }
            }
        }
    }

    if let Some(h) = current {
        hosts.push(h);
    }

    Ok(hosts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_config() {
        let config = r#"
Host server1
    HostName 192.168.1.10
    User admin
    Port 2222
    IdentityFile ~/.ssh/id_rsa

Host dev-*
    User developer

Host bastion
    HostName bastion.example.com
    ProxyJump jump-host
"#;
        let hosts = parse_ssh_config_str(config).unwrap();
        assert_eq!(hosts.len(), 2); // dev-* is skipped (wildcard)
        assert_eq!(hosts[0].name, "server1");
        assert_eq!(hosts[0].hostname.as_deref(), Some("192.168.1.10"));
        assert_eq!(hosts[0].port, Some(2222));
        assert_eq!(hosts[1].name, "bastion");
        assert_eq!(hosts[1].proxy_jump.as_deref(), Some("jump-host"));
    }
}
