use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use tokio::sync::mpsc;

use tp_core::workspace::PanelConfig;

/// Events emitted by a PTY.
#[derive(Debug)]
pub enum PtyEvent {
    Output { panel_id: String, data: Vec<u8> },
    Exited { panel_id: String, code: Option<u32> },
}

/// Handle to a running PTY process.
pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    /// Write bytes to the PTY (user input).
    pub fn write_input(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Resize the PTY.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

/// Manages all PTY instances.
pub struct PtyManager {
    handles: HashMap<String, PtyHandle>,
    event_tx: mpsc::UnboundedSender<PtyEvent>,
}

impl PtyManager {
    pub fn new(event_tx: mpsc::UnboundedSender<PtyEvent>) -> Self {
        Self {
            handles: HashMap::new(),
            event_tx,
        }
    }

    /// Spawn a local shell PTY for a panel.
    pub fn spawn_local(
        &mut self,
        panel: &PanelConfig,
        cols: u16,
        rows: u16,
        shell: &str,
    ) -> Result<()> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        let mut cmd = CommandBuilder::new(shell);
        if let Some(ref cwd) = panel.cwd {
            cmd.cwd(cwd);
        }
        for (k, v) in &panel.env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave); // Close slave side in parent

        let writer = pair.master.take_writer()?;
        let mut reader = pair.master.try_clone_reader()?;

        let panel_id = panel.id.clone();
        let event_tx = self.event_tx.clone();

        // Reader thread: read PTY output and forward to event channel
        let read_panel_id = panel_id.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = event_tx.send(PtyEvent::Exited {
                            panel_id: read_panel_id,
                            code: None,
                        });
                        break;
                    }
                    Ok(n) => {
                        let _ = event_tx.send(PtyEvent::Output {
                            panel_id: read_panel_id.clone(),
                            data: buf[..n].to_vec(),
                        });
                    }
                    Err(_) => {
                        let _ = event_tx.send(PtyEvent::Exited {
                            panel_id: read_panel_id,
                            code: None,
                        });
                        break;
                    }
                }
            }
        });

        let handle = PtyHandle {
            master: pair.master,
            writer,
            _child: child,
        };

        self.handles.insert(panel_id, handle);
        Ok(())
    }

    /// Send startup commands to a panel.
    pub fn send_startup_commands(&mut self, panel_id: &str, commands: &[String]) -> Result<()> {
        if let Some(handle) = self.handles.get_mut(panel_id) {
            for cmd in commands {
                let line = format!("{}\n", cmd);
                handle.write_input(line.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Write input to a specific panel.
    pub fn write_to_panel(&mut self, panel_id: &str, data: &[u8]) -> Result<()> {
        if let Some(handle) = self.handles.get_mut(panel_id) {
            handle.write_input(data)?;
        }
        Ok(())
    }

    /// Resize a panel's PTY.
    pub fn resize_panel(&self, panel_id: &str, cols: u16, rows: u16) -> Result<()> {
        if let Some(handle) = self.handles.get(panel_id) {
            handle.resize(cols, rows)?;
        }
        Ok(())
    }

    /// Get mutable handle by panel ID.
    pub fn get_mut(&mut self, panel_id: &str) -> Option<&mut PtyHandle> {
        self.handles.get_mut(panel_id)
    }

    /// Remove a panel's PTY.
    pub fn remove(&mut self, panel_id: &str) -> Option<PtyHandle> {
        self.handles.remove(panel_id)
    }

    /// Get all active panel IDs.
    pub fn active_panels(&self) -> Vec<&str> {
        self.handles.keys().map(|s| s.as_str()).collect()
    }
}
