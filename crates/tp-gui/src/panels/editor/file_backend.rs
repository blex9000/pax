//! # File Backend Abstraction
//!
//! Abstracts filesystem operations so the code editor can work transparently
//! on both local directories and remote servers via SSH.
//!
//! - `LocalFileBackend`: standard `std::fs` operations + local `git` commands
//! - `SshFileBackend`: executes commands on the remote host via SSH with
//!   ControlMaster for persistent connection multiplexing

use std::path::{Path, PathBuf};

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Entry from a directory listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_ignored: bool,
}

/// Abstraction over local and remote file operations.
///
/// Designed to be implementation-agnostic: currently backed by Local (std::fs)
/// and SSH (shell commands), but the trait is ready for a future Agent-based
/// backend that runs a binary on the remote host for faster batch operations.
pub trait FileBackend: std::fmt::Debug + Send + Sync {
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

    /// Delete a directory recursively.
    fn delete_dir(&self, path: &Path) -> Result<(), String>;

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
        let output = self.git_command(&["grep", "-n", "--no-color", "-i", "--", pattern])?;
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
        if self.is_remote() {
            "ssh"
        } else {
            "local"
        }
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
        Self {
            root_dir: root_dir.to_path_buf(),
        }
    }
}

impl FileBackend for LocalFileBackend {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, String> {
        let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        let mut raw_entries = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            raw_entries.push((name, is_dir));
        }
        let ignored = git_ignored_names_local(&self.root_dir, path, &raw_entries)?;
        let mut result = Vec::new();
        for (name, is_dir) in raw_entries {
            result.push(DirEntry {
                is_ignored: ignored.contains(&name),
                name,
                is_dir,
            });
        }
        result.sort_by(|a, b| {
            // Directories first, then alphabetical
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
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

    fn delete_dir(&self, path: &Path) -> Result<(), String> {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    }

    fn rename_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| e.to_string())
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
    /// True only after ControlMaster background thread confirms connection.
    /// AtomicBool because it's set from a background std::thread.
    connected: std::sync::Arc<std::sync::atomic::AtomicBool>,
    connecting: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SshFileBackend {
    pub fn new(
        root_dir: &str,
        host: &str,
        port: u16,
        user: &str,
        password: Option<&str>,
        identity_file: Option<&str>,
    ) -> Self {
        let control_path = format!(
            "/tmp/pax_ssh_{}_{}",
            host.replace('.', "_"),
            std::process::id()
        );

        let connected = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let connecting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let backend = Self {
            root_dir: PathBuf::from(root_dir),
            host: host.to_string(),
            user: user.to_string(),
            port,
            password: password.map(|s| s.to_string()),
            identity_file: identity_file
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            control_path,
            connected,
            connecting,
        };

        // Establish the ControlMaster connection in background
        backend.setup_control_master();

        backend
    }

    /// Establish a persistent SSH ControlMaster connection.
    /// Non-blocking: spawns SSH in a background std::thread.
    /// Sets `connected` flag to true only after successful connection.
    fn setup_control_master(&self) {
        if self
            .connecting
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        let flag = self.connected.clone();
        let connecting = self.connecting.clone();
        let mut cmd = self.base_ssh_command();
        cmd.args(["-fNM"]); // fork to background, no command, master mode
        let label = format!("{}@{}", self.user, self.host);
        std::thread::spawn(move || {
            match cmd.status() {
                Ok(s) if s.success() => {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    tracing::info!("SSH ControlMaster connected: {}", label);
                }
                Ok(s) => {
                    flag.store(false, std::sync::atomic::Ordering::Relaxed);
                    tracing::warn!("SSH ControlMaster failed for {}: exit {}", label, s);
                }
                Err(e) => {
                    flag.store(false, std::sync::atomic::Ordering::Relaxed);
                    tracing::warn!("SSH ControlMaster spawn failed for {}: {}", label, e);
                }
            }
            connecting.store(false, std::sync::atomic::Ordering::SeqCst);
        });
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
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "ConnectTimeout=3",
            "-o",
            &format!("ControlPath={}", self.control_path),
            "-o",
            "ControlMaster=auto",
            "-o",
            "ControlPersist=300",
            "-p",
            &self.port.to_string(),
        ]);

        if let Some(ref key) = self.identity_file {
            cmd.args(["-i", key]);
        }

        cmd.arg(&format!("{}@{}", self.user, self.host));
        cmd
    }

    /// Check if SSH connection is ready. Zero-cost: just reads an atomic bool.
    fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Execute a remote command and return stdout.
    /// Returns Err immediately if SSH connection is not ready.
    fn ssh_exec(&self, remote_cmd: &str) -> Result<String, String> {
        if !self.is_connected() {
            self.setup_control_master();
            return Err("SSH not connected yet".to_string());
        }
        let mut cmd = self.base_ssh_command();
        cmd.arg(remote_cmd);
        let output = cmd
            .output()
            .map_err(|e| format!("SSH exec failed: {}", e))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if stderr.contains("Connection") || stderr.contains("closed") {
                self.connected
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                self.setup_control_master();
            }
            Err(if stderr.is_empty() {
                format!("exit {}", output.status)
            } else {
                stderr
            })
        }
    }

    /// Execute a command that needs stdin (for write_file).
    fn ssh_exec_with_stdin(&self, remote_cmd: &str, input: &str) -> Result<(), String> {
        if !self.is_connected() {
            self.setup_control_master();
            return Err("SSH not connected yet".to_string());
        }
        use std::io::Write;
        let mut cmd = self.base_ssh_command();
        cmd.arg(remote_cmd);
        cmd.stdin(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("SSH spawn failed: {}", e))?;
        if let Some(ref mut stdin) = child.stdin {
            stdin
                .write_all(input.as_bytes())
                .map_err(|e| format!("Write stdin: {}", e))?;
        }
        let status = child.wait().map_err(|e| format!("SSH wait: {}", e))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("SSH command failed: {}", status))
        }
    }
}

impl FileBackend for SshFileBackend {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, String> {
        let path_str = path.to_string_lossy();
        // ls -1Ap lists entries one per line, includes hidden files except . and ..
        let output = self.ssh_exec(&format!("ls -1Ap {}", shell_quote(&path_str)))?;
        let mut raw_entries = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.ends_with('/') {
                raw_entries.push((line.trim_end_matches('/').to_string(), true));
            } else {
                raw_entries.push((line.to_string(), false));
            }
        }
        let ignored = git_ignored_names_remote(self, path, &raw_entries).unwrap_or_default();
        let mut result = Vec::new();
        for (name, is_dir) in raw_entries {
            result.push(DirEntry {
                is_ignored: ignored.contains(&name),
                name,
                is_dir,
            });
        }
        // Directories first, then alphabetical
        result.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(result)
    }

    fn read_file(&self, path: &Path) -> Result<String, String> {
        self.ssh_exec(&format!("cat {}", shell_quote(&path.to_string_lossy())))
    }

    fn write_file(&self, path: &Path, content: &str) -> Result<(), String> {
        self.ssh_exec_with_stdin(
            &format!("cat > {}", shell_quote(&path.to_string_lossy())),
            content,
        )
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.ssh_exec(&format!(
            "test -e {} && echo yes",
            shell_quote(&path.to_string_lossy())
        ))
        .map(|s| s.trim() == "yes")
        .unwrap_or(false)
    }

    fn delete_file(&self, path: &Path) -> Result<(), String> {
        self.ssh_exec(&format!("rm -f {}", shell_quote(&path.to_string_lossy())))
            .map(|_| ())
    }

    fn delete_dir(&self, path: &Path) -> Result<(), String> {
        self.ssh_exec(&format!("rm -rf {}", shell_quote(&path.to_string_lossy())))
            .map(|_| ())
    }

    fn rename_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        self.ssh_exec(&format!(
            "mv {} {}",
            shell_quote(&from.to_string_lossy()),
            shell_quote(&to.to_string_lossy()),
        ))
        .map(|_| ())
    }

    fn copy_file(&self, from: &Path, to: &Path) -> Result<(), String> {
        self.ssh_exec(&format!(
            "cp {} {}",
            shell_quote(&from.to_string_lossy()),
            shell_quote(&to.to_string_lossy()),
        ))
        .map(|_| ())
    }

    fn create_dir(&self, path: &Path) -> Result<(), String> {
        self.ssh_exec(&format!(
            "mkdir -p {}",
            shell_quote(&path.to_string_lossy())
        ))
        .map(|_| ())
    }

    fn git_command(&self, args: &[&str]) -> Result<String, String> {
        let git_args = args
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        self.ssh_exec(&format!(
            "cd {} && git {}",
            shell_quote(&self.root_dir.to_string_lossy()),
            git_args,
        ))
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
        // Close the ControlMaster connection using proper options
        let mut cmd = if self.password.is_some() {
            let mut c = std::process::Command::new("sshpass");
            c.args(["-p", self.password.as_deref().unwrap_or(""), "ssh"]);
            c
        } else {
            std::process::Command::new("ssh")
        };
        cmd.args([
            "-o",
            &format!("ControlPath={}", self.control_path),
            "-O",
            "exit",
        ]);
        if let Some(ref key) = self.identity_file {
            cmd.args(["-i", key]);
        }
        cmd.arg(&format!("{}@{}", self.user, self.host));
        let _ = cmd.status();
    }
}

fn relative_git_path(root: &Path, dir: &Path, name: &str) -> Option<String> {
    let relative_dir = dir.strip_prefix(root).ok()?;
    let relative_path = if relative_dir.as_os_str().is_empty() {
        PathBuf::from(name)
    } else {
        relative_dir.join(name)
    };
    Some(relative_path.to_string_lossy().to_string())
}

fn parse_ignored_names_from_git_output(output: &str) -> std::collections::HashSet<String> {
    output
        .split('\0')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            Path::new(part)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .collect()
}

fn git_ignored_names_local(
    root: &Path,
    dir: &Path,
    entries: &[(String, bool)],
) -> Result<std::collections::HashSet<String>, String> {
    let relative_paths: Vec<String> = entries
        .iter()
        .filter_map(|(name, _)| relative_git_path(root, dir, name))
        .collect();
    if relative_paths.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    use std::io::Write;
    let mut cmd = std::process::Command::new("git");
    cmd.args(["check-ignore", "--stdin", "-z"])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(stdin) = child.stdin.as_mut() {
        for relative_path in &relative_paths {
            stdin
                .write_all(relative_path.as_bytes())
                .and_then(|_| stdin.write_all(&[0]))
                .map_err(|e| e.to_string())?;
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.code() == Some(128) {
        return Ok(std::collections::HashSet::new());
    }
    Ok(parse_ignored_names_from_git_output(
        &String::from_utf8_lossy(&output.stdout),
    ))
}

fn git_ignored_names_remote(
    backend: &SshFileBackend,
    dir: &Path,
    entries: &[(String, bool)],
) -> Result<std::collections::HashSet<String>, String> {
    let relative_paths: Vec<String> = entries
        .iter()
        .filter_map(|(name, _)| relative_git_path(&backend.root_dir, dir, name))
        .collect();
    if relative_paths.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    let args = relative_paths
        .iter()
        .map(|path| shell_quote(path))
        .collect::<Vec<_>>()
        .join(" ");
    let command = format!(
        "cd {} && printf '%s\\0' {} | git check-ignore --stdin -z 2>/dev/null || true",
        shell_quote(&backend.root_dir.to_string_lossy()),
        args,
    );
    let output = backend.ssh_exec(&command)?;
    Ok(parse_ignored_names_from_git_output(&output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[derive(Debug)]
    struct MockBackend {
        args: Mutex<Vec<String>>,
        output: String,
    }

    impl FileBackend for MockBackend {
        fn list_dir(&self, _path: &Path) -> Result<Vec<DirEntry>, String> {
            unreachable!()
        }
        fn read_file(&self, _path: &Path) -> Result<String, String> {
            unreachable!()
        }
        fn write_file(&self, _path: &Path, _content: &str) -> Result<(), String> {
            unreachable!()
        }
        fn file_exists(&self, _path: &Path) -> bool {
            false
        }
        fn delete_file(&self, _path: &Path) -> Result<(), String> {
            unreachable!()
        }
        fn delete_dir(&self, _path: &Path) -> Result<(), String> {
            unreachable!()
        }
        fn rename_file(&self, _from: &Path, _to: &Path) -> Result<(), String> {
            unreachable!()
        }
        fn copy_file(&self, _from: &Path, _to: &Path) -> Result<(), String> {
            unreachable!()
        }
        fn create_dir(&self, _path: &Path) -> Result<(), String> {
            unreachable!()
        }
        fn git_command(&self, args: &[&str]) -> Result<String, String> {
            *self.args.lock().unwrap() = args.iter().map(|s| s.to_string()).collect();
            Ok(self.output.clone())
        }
        fn root(&self) -> &Path {
            Path::new(".")
        }
        fn is_remote(&self) -> bool {
            false
        }
    }

    #[test]
    fn search_files_uses_case_insensitive_git_grep() {
        let backend = MockBackend {
            args: Mutex::new(Vec::new()),
            output: "src/main.rs:12:Hello World".to_string(),
        };

        let results = backend.search_files("hello").unwrap();

        assert_eq!(
            backend.args.lock().unwrap().as_slice(),
            ["grep", "-n", "--no-color", "-i", "--", "hello"]
        );
        assert_eq!(
            results,
            vec![("src/main.rs".to_string(), 12, "Hello World".to_string())]
        );
    }

    #[test]
    fn local_list_dir_includes_hidden_and_marks_gitignored_entries() {
        let dir = tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored.log\nignored_dir/\n").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        std::fs::write(dir.path().join("visible.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("ignored.log"), "ignore me\n").unwrap();
        std::fs::create_dir_all(dir.path().join("ignored_dir")).unwrap();

        let backend = LocalFileBackend::new(dir.path());
        let entries = backend.list_dir(dir.path()).unwrap();

        assert!(entries
            .iter()
            .any(|entry| entry.name == ".env" && !entry.is_ignored));
        assert!(entries
            .iter()
            .any(|entry| entry.name == "visible.rs" && !entry.is_ignored));
        assert!(entries
            .iter()
            .any(|entry| entry.name == "ignored.log" && entry.is_ignored));
        assert!(entries
            .iter()
            .any(|entry| entry.name == "ignored_dir" && entry.is_ignored && entry.is_dir));
    }

    #[test]
    fn local_delete_dir_removes_non_empty_directory() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("src/nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("main.rs"), "fn main() {}\n").unwrap();

        let backend = LocalFileBackend::new(dir.path());
        let target = dir.path().join("src");
        assert!(target.exists());

        backend.delete_dir(&target).unwrap();

        assert!(!target.exists());
    }
}
