//! # File Backend Abstraction
//!
//! Abstracts filesystem operations so the code editor can work transparently
//! on both local directories and remote servers via SSH.
//!
//! - `LocalFileBackend`: standard `std::fs` operations + local `git` commands
//! - `SshFileBackend`: executes commands on the remote host via SSH with
//!   ControlMaster for persistent connection multiplexing

use std::path::{Path, PathBuf};

/// Entry from a directory listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Abstraction over local and remote file operations.
///
/// Designed to be implementation-agnostic: currently backed by Local (std::fs)
/// and SSH (shell commands), but the trait is ready for a future Agent-based
/// backend that runs a binary on the remote host for faster batch operations.
pub trait FileBackend: std::fmt::Debug {
    /// List entries in a directory (files and subdirectories).
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, String>;

    /// Read a file's contents as a UTF-8 string.
    fn read_file(&self, path: &Path) -> Result<String, String>;

    /// Write content to a file (creates or overwrites).
    fn write_file(&self, path: &Path, content: &str) -> Result<(), String>;

    /// Check if a file or directory exists.
    fn file_exists(&self, path: &Path) -> bool;

    /// Delete a file.
    fn delete_file(&self, path: &Path) -> Result<(), String>;

    /// Rename/move a file.
    fn rename_file(&self, from: &Path, to: &Path) -> Result<(), String>;

    /// Copy a file.
    fn copy_file(&self, from: &Path, to: &Path) -> Result<(), String>;

    /// Create a directory (and parents).
    fn create_dir(&self, path: &Path) -> Result<(), String>;

    /// Run a git command in the project root. Returns stdout.
    fn git_command(&self, args: &[&str]) -> Result<String, String>;

    /// Get file content at a specific git ref (e.g. "HEAD:path/file.rs").
    fn git_show(&self, spec: &str) -> Result<String, String> {
        self.git_command(&["show", spec])
    }

    /// Search files for a pattern. Returns Vec<(relative_path, line_number, line_content)>.
    /// Default uses git grep; backends can override for non-git projects.
    fn search_files(&self, pattern: &str) -> Result<Vec<(String, usize, String)>, String> {
        let output = self.git_command(&["grep", "-n", "--no-color", pattern])?;
        let mut results = Vec::new();
        for line in output.lines() {
            // format: path:line_num:content
            if let Some(colon1) = line.find(':') {
                let path = &line[..colon1];
                let rest = &line[colon1 + 1..];
                if let Some(colon2) = rest.find(':') {
                    if let Ok(line_num) = rest[..colon2].parse::<usize>() {
                        let content = &rest[colon2 + 1..];
                        results.push((path.to_string(), line_num, content.to_string()));
                    }
                }
            }
        }
        Ok(results)
    }

    /// Root directory of the project (local path or remote path).
    fn root(&self) -> &Path;

    /// Whether this is a remote backend.
    fn is_remote(&self) -> bool;

    /// Backend type identifier (for logging/UI).
    fn backend_type(&self) -> &str {
        if self.is_remote() { "ssh" } else { "local" }
    }
}

// ── Local Backend ───────────────────────────────────────────────────────────

/// Local filesystem backend — wraps std::fs and local git.
#[derive(Debug)]
pub struct LocalFileBackend {
    root_dir: PathBuf,
}

impl LocalFileBackend {
    pub fn new(root_dir: &Path) -> Self {
        Self { root_dir: root_dir.to_path_buf() }
    }
}

impl FileBackend for LocalFileBackend {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, String> {
        let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            result.push(DirEntry { name, is_dir });
        }
        result.sort_by(|a, b| {
            // Directories first, then alphabetical
            b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(result)
    }

    fn read_file(&self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }

    fn write_file(&self, path: &Path, content: &str) -> Result<(), String> {
        std::fs::write(path, content).map_err(|e| e.to_string())
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn delete_file(&self, path: &Path) -> Result<(), String> {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }

    fn rename_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::copy(from, to).map(|_| ()).map_err(|e| e.to_string())
    }

    fn create_dir(&self, path: &Path) -> Result<(), String> {
        std::fs::create_dir_all(path).map_err(|e| e.to_string())
    }

    fn git_command(&self, args: &[&str]) -> Result<String, String> {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&self.root_dir)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }

    fn root(&self) -> &Path {
        &self.root_dir
    }

    fn is_remote(&self) -> bool {
        false
    }
}

// ── SSH Backend ─────────────────────────────────────────────────────────────

/// Remote SSH backend — executes commands on the remote host.
/// Uses SSH ControlMaster for connection multiplexing (one handshake,
/// all subsequent commands reuse the socket).
#[derive(Debug)]
pub struct SshFileBackend {
    root_dir: PathBuf,
    host: String,
    user: String,
    port: u16,
    password: Option<String>,
    identity_file: Option<String>,
    control_path: String,
}

impl SshFileBackend {
    pub fn new(
        root_dir: &str,
        host: &str, port: u16, user: &str,
        password: Option<&str>, identity_file: Option<&str>,
    ) -> Self {
        let control_path = format!("/tmp/pax_ssh_{}_{}", host.replace('.', "_"), std::process::id());

        let backend = Self {
            root_dir: PathBuf::from(root_dir),
            host: host.to_string(),
            user: user.to_string(),
            port,
            password: password.map(|s| s.to_string()),
            identity_file: identity_file.filter(|s| !s.is_empty()).map(|s| s.to_string()),
            control_path,
        };

        // Establish the ControlMaster connection in background
        backend.setup_control_master();

        backend
    }

    /// Establish a persistent SSH ControlMaster connection.
    fn setup_control_master(&self) {
        let mut cmd = self.base_ssh_command();
        cmd.args(["-fNM"]); // fork to background, no command, master mode
        let _ = self.run_with_password(&mut cmd);
    }

    /// Build base SSH command with common options.
    fn base_ssh_command(&self) -> std::process::Command {
        let mut cmd = if self.password.is_some() {
            let mut c = std::process::Command::new("sshpass");
            c.args(["-p", self.password.as_deref().unwrap_or(""), "ssh"]);
            c
        } else {
            std::process::Command::new("ssh")
        };

        cmd.args([
            "-o", "StrictHostKeyChecking=no",
            "-o", "ConnectTimeout=10",
            "-o", &format!("ControlPath={}", self.control_path),
            "-o", "ControlMaster=auto",
            "-o", "ControlPersist=300",
            "-p", &self.port.to_string(),
        ]);

        if let Some(ref key) = self.identity_file {
            cmd.args(["-i", key]);
        }

        cmd.arg(&format!("{}@{}", self.user, self.host));
        cmd
    }

    /// Execute a remote command and return stdout.
    fn ssh_exec(&self, remote_cmd: &str) -> Result<String, String> {
        let mut cmd = self.base_ssh_command();
        cmd.arg(remote_cmd);
        let output = cmd.output().map_err(|e| format!("SSH exec failed: {}", e))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(if stderr.is_empty() { format!("exit {}", output.status) } else { stderr })
        }
    }

    /// Execute a command that needs stdin (for write_file).
    fn ssh_exec_with_stdin(&self, remote_cmd: &str, input: &str) -> Result<(), String> {
        use std::io::Write;
        let mut cmd = self.base_ssh_command();
        cmd.arg(remote_cmd);
        cmd.stdin(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("SSH spawn failed: {}", e))?;
        if let Some(ref mut stdin) = child.stdin {
            stdin.write_all(input.as_bytes()).map_err(|e| format!("Write stdin: {}", e))?;
        }
        let status = child.wait().map_err(|e| format!("SSH wait: {}", e))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("SSH command failed: {}", status))
        }
    }

    /// Run a command, handling password via sshpass if needed.
    fn run_with_password(&self, cmd: &mut std::process::Command) -> Result<(), String> {
        let status = cmd.status().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err(format!("exit {}", status)) }
    }
}

impl FileBackend for SshFileBackend {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, String> {
        let path_str = path.to_string_lossy();
        // ls -1ap lists entries one per line, dirs have trailing /
        let output = self.ssh_exec(&format!("ls -1ap '{}'", path_str))?;
        let mut result = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() || line == "./" || line == "../" { continue; }
            if line.ends_with('/') {
                result.push(DirEntry {
                    name: line.trim_end_matches('/').to_string(),
                    is_dir: true,
                });
            } else {
                result.push(DirEntry {
                    name: line.to_string(),
                    is_dir: false,
                });
            }
        }
        // Directories first, then alphabetical
        result.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(result)
    }

    fn read_file(&self, path: &Path) -> Result<String, String> {
        self.ssh_exec(&format!("cat '{}'", path.to_string_lossy()))
    }

    fn write_file(&self, path: &Path, content: &str) -> Result<(), String> {
        self.ssh_exec_with_stdin(
            &format!("cat > '{}'", path.to_string_lossy()),
            content,
        )
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.ssh_exec(&format!("test -e '{}' && echo yes", path.to_string_lossy()))
            .map(|s| s.trim() == "yes")
            .unwrap_or(false)
    }

    fn delete_file(&self, path: &Path) -> Result<(), String> {
        self.ssh_exec(&format!("rm -f '{}'", path.to_string_lossy())).map(|_| ())
    }

    fn rename_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        self.ssh_exec(&format!("mv '{}' '{}'", from.to_string_lossy(), to.to_string_lossy())).map(|_| ())
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        self.ssh_exec(&format!("cp '{}' '{}'", from.to_string_lossy(), to.to_string_lossy())).map(|_| ())
    }

    fn create_dir(&self, path: &Path) -> Result<(), String> {
        self.ssh_exec(&format!("mkdir -p '{}'", path.to_string_lossy())).map(|_| ())
    }

    fn git_command(&self, args: &[&str]) -> Result<String, String> {
        let git_args = args.iter()
            .map(|a| format!("'{}'", a))
            .collect::<Vec<_>>()
            .join(" ");
        self.ssh_exec(&format!("cd '{}' && git {}", self.root_dir.to_string_lossy(), git_args))
    }

    fn root(&self) -> &Path {
        &self.root_dir
    }

    fn is_remote(&self) -> bool {
        true
    }
}

impl Drop for SshFileBackend {
    fn drop(&mut self) {
        // Close the ControlMaster connection
        let _ = std::process::Command::new("ssh")
            .args([
                "-o", &format!("ControlPath={}", self.control_path),
                "-O", "exit",
                &format!("{}@{}", self.user, self.host),
            ])
            .status();
    }
}
