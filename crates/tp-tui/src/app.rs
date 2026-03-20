use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Terminal;
use std::collections::{HashMap, HashSet};
use std::io::stdout;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use tp_core::alert::{self, CompiledAlert};
use tp_core::ssh;
use tp_core::workspace::Workspace;
use tp_db::Database;
use tp_pty::manager::{PtyEvent, PtyManager};
use tp_pty::multiplexer::{self, BroadcastResult};
use tp_pty::output::OutputBuffer;

use crate::layout::ResolvedLayout;
use crate::panels::terminal::TerminalPanel;
use crate::widgets::palette::{CommandPalette, PaletteAction, PaletteItem};

/// Application mode.
#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Normal,
    Broadcast(String), // group name
    Palette,
    Zoom,
}

/// Per-panel state.
struct PanelState {
    terminal: TerminalPanel,
    output_buf: OutputBuffer,
}

/// Main application state.
pub struct App {
    workspace: Workspace,
    panels: HashMap<String, PanelState>,
    focus_order: Vec<String>,
    focus_index: usize,
    mode: Mode,
    zoomed_panel: Option<String>,
    alerts: Vec<CompiledAlert>,
    palette: CommandPalette,
    status_message: Option<String>,
    should_quit: bool,
    recording: HashSet<String>,
    db: Option<Database>,
    last_db_flush: Instant,
    active_tab: HashMap<String, usize>,
}

impl App {
    pub fn new(workspace: Workspace) -> Result<Self> {
        let alerts = alert::compile_alerts(&workspace.alerts)?;

        let palette_items = build_palette_items(&workspace);
        let palette = CommandPalette::new(palette_items);

        let focus_order: Vec<String> = workspace.layout.panel_ids().iter().map(|s| s.to_string()).collect();

        // Initialize recording set from panel configs
        let recording: HashSet<String> = workspace
            .panels
            .iter()
            .filter(|p| p.record_output)
            .map(|p| p.id.clone())
            .collect();

        // Open database
        let db = {
            let db_path = Database::default_path();
            Database::open(&db_path).ok()
        };

        Ok(Self {
            workspace,
            panels: HashMap::new(),
            focus_order,
            focus_index: 0,
            mode: Mode::Normal,
            zoomed_panel: None,
            alerts,
            palette,
            status_message: None,
            should_quit: false,
            recording,
            db,
            last_db_flush: Instant::now(),
            active_tab: HashMap::new(),
        })
    }

    fn focused_panel_id(&self) -> Option<&str> {
        self.focus_order.get(self.focus_index).map(|s| s.as_str())
    }

    fn focus_next(&mut self) {
        if !self.focus_order.is_empty() {
            self.focus_index = (self.focus_index + 1) % self.focus_order.len();
        }
    }

    fn focus_prev(&mut self) {
        if !self.focus_order.is_empty() {
            self.focus_index = if self.focus_index == 0 {
                self.focus_order.len() - 1
            } else {
                self.focus_index - 1
            };
        }
    }

    fn toggle_zoom(&mut self) {
        if self.zoomed_panel.is_some() {
            self.zoomed_panel = None;
            self.mode = Mode::Normal;
        } else {
            self.zoomed_panel = self.focused_panel_id().map(|s| s.to_string());
            self.mode = Mode::Zoom;
        }
    }

    /// Main entry point: initialize terminal, spawn PTYs, run event loop.
    pub async fn run(mut self) -> Result<()> {
        // Execute workspace-level startup script (blocking)
        if let Some(ref script) = self.workspace.startup_script {
            tracing::info!("Running workspace startup script");
            let status = std::process::Command::new("bash")
                .arg("-c")
                .arg(script)
                .status();
            match status {
                Ok(s) if !s.success() => {
                    anyhow::bail!("Workspace startup script failed with {}", s);
                }
                Err(e) => {
                    anyhow::bail!("Workspace startup script error: {}", e);
                }
                _ => {}
            }
        }

        // Setup terminal
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        // PTY event channel
        let (pty_tx, mut pty_rx) = mpsc::unbounded_channel::<PtyEvent>();
        let mut pty_mgr = PtyManager::new(pty_tx);

        // Get initial terminal size
        let size = terminal.size()?;

        // Resolve layout to get panel sizes
        let resolved = ResolvedLayout::resolve(
            &self.workspace.layout,
            Rect {
                x: 0,
                y: 0,
                width: size.width,
                height: size.height.saturating_sub(1),
            },
            &self.active_tab,
        );

        // Spawn PTYs for all panels
        let shell = self.workspace.settings.default_shell.clone();
        let mut post_scripts: Vec<String> = Vec::new();

        for panel_cfg in &self.workspace.panels {
            // Execute pre-script (blocking) — skip panel on failure
            if let Some(ref pre) = panel_cfg.pre_script {
                tracing::info!("Running pre-script for panel '{}'", panel_cfg.id);
                let status = std::process::Command::new("bash")
                    .arg("-c")
                    .arg(pre)
                    .status();
                match status {
                    Ok(s) if !s.success() => {
                        tracing::warn!(
                            "Pre-script for panel '{}' failed ({}), skipping panel",
                            panel_cfg.id, s
                        );
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Pre-script for panel '{}' error: {}, skipping panel",
                            panel_cfg.id, e
                        );
                        continue;
                    }
                    _ => {}
                }
            }

            let rect = resolved.get(&panel_cfg.id).unwrap_or(Rect::new(0, 0, 80, 24));
            let cols = rect.width.saturating_sub(2).max(1);
            let rows = rect.height.saturating_sub(2).max(1);

            pty_mgr.spawn_local(panel_cfg, cols, rows, &shell)?;

            if !panel_cfg.startup_commands.is_empty() {
                pty_mgr.send_startup_commands(&panel_cfg.id, &panel_cfg.startup_commands)?;
            }

            // Collect post-scripts for async execution
            if let Some(ref post) = panel_cfg.post_script {
                post_scripts.push(post.clone());
            }

            let state = PanelState {
                terminal: TerminalPanel::new(rows, cols),
                output_buf: OutputBuffer::new(self.workspace.settings.scrollback_lines),
            };
            self.panels.insert(panel_cfg.id.clone(), state);
        }

        // Execute post-scripts asynchronously
        for script in post_scripts {
            tokio::spawn(async move {
                let _ = tokio::process::Command::new("bash")
                    .arg("-c")
                    .arg(&script)
                    .status()
                    .await;
            });
        }

        // Event loop
        loop {
            // Drain PTY events
            while let Ok(evt) = pty_rx.try_recv() {
                match evt {
                    PtyEvent::Output { panel_id, data } => {
                        if let Some(state) = self.panels.get_mut(&panel_id) {
                            state.terminal.feed(&data);

                            // Get panel groups for alert matching
                            let groups: Vec<String> = self.workspace
                                .panel(&panel_id)
                                .map(|p| p.groups.clone())
                                .unwrap_or_default();

                            let triggered = state.output_buf.feed(
                                &data,
                                &panel_id,
                                &groups,
                                &self.alerts,
                            );

                            // Apply alert actions
                            for alert in &triggered {
                                for action in &alert.actions {
                                    match action {
                                        tp_core::workspace::AlertAction::BorderColor(color) => {
                                            let c = color_from_name(color);
                                            state.terminal.alert_border_color = Some(c);
                                        }
                                        tp_core::workspace::AlertAction::DesktopNotification => {
                                            let summary = format!("myterms [{}]", panel_id);
                                            let body = alert.line[..alert.line.len().min(200)].to_string();
                                            // Fire notification in background
                                            tokio::spawn(async move {
                                                let _ = notify_rust::Notification::new()
                                                    .summary(&summary)
                                                    .body(&body)
                                                    .timeout(5000)
                                                    .show();
                                            });
                                            self.status_message = Some(format!(
                                                "ALERT [{}]: {}",
                                                panel_id,
                                                &alert.line[..alert.line.len().min(60)]
                                            ));
                                        }
                                        tp_core::workspace::AlertAction::Sound => {
                                            // Bell character to terminal
                                            let _ = std::io::Write::write_all(
                                                &mut std::io::stdout(),
                                                b"\x07",
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    PtyEvent::Exited { panel_id, .. } => {
                        self.status_message = Some(format!("Panel '{}' exited", panel_id));
                    }
                }
            }

            // Render
            terminal.draw(|frame| {
                let full_area = frame.area();
                let main_area = Rect {
                    x: 0,
                    y: 0,
                    width: full_area.width,
                    height: full_area.height.saturating_sub(1),
                };
                let status_area = Rect {
                    x: 0,
                    y: full_area.height.saturating_sub(1),
                    width: full_area.width,
                    height: 1,
                };

                let focused_id = self.focused_panel_id().unwrap_or("").to_string();

                if let Some(ref zoomed_id) = self.zoomed_panel {
                    // Full-page zoom: render only the zoomed panel
                    if let Some(state) = self.panels.get(zoomed_id) {
                        let name = self.workspace
                            .panel(zoomed_id)
                            .map(|p| p.name.as_str())
                            .unwrap_or(zoomed_id);
                        let title = format!(" {} [ZOOM] ", name);
                        state.terminal.render(
                            main_area,
                            frame.buffer_mut(),
                            &title,
                            true,
                        );
                    }
                } else {
                    // Normal layout
                    let resolved = ResolvedLayout::resolve(&self.workspace.layout, main_area, &self.active_tab);
                    for (panel_id, rect) in &resolved.panels {
                        if let Some(state) = self.panels.get(panel_id) {
                            let is_focused = panel_id == &focused_id;
                            let name = self.workspace
                                .panel(panel_id)
                                .map(|p| p.name.as_str())
                                .unwrap_or(panel_id);

                            let mut title = format!(" {} ", name);
                            if let Mode::Broadcast(ref g) = self.mode {
                                if self.workspace
                                    .panel(panel_id)
                                    .map(|p| p.groups.iter().any(|pg| pg == g))
                                    .unwrap_or(false)
                                {
                                    title = format!(" {} [BC:{}] ", name, g);
                                }
                            }

                            state.terminal.render(
                                *rect,
                                frame.buffer_mut(),
                                &title,
                                is_focused,
                            );
                        }
                    }
                }

                // Status bar
                let status_text = self.build_status_line();
                let status = Paragraph::new(Line::from(status_text))
                    .style(Style::default().bg(Color::DarkGray).fg(Color::White));
                status.render(status_area, frame.buffer_mut());

                // Command palette overlay
                self.palette.render(main_area, frame.buffer_mut());
            })?;

            // Handle input
            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) => {
                        if self.palette.visible {
                            self.handle_palette_key(key, &mut pty_mgr)?;
                        } else {
                            self.handle_key(key, &mut pty_mgr)?;
                        }
                    }
                    Event::Resize(w, h) => {
                        let main_h = h.saturating_sub(1);
                        let main_area = Rect::new(0, 0, w, main_h);

                        if let Some(ref zoomed_id) = self.zoomed_panel {
                            // Resize zoomed panel to full area
                            let cols = main_area.width.saturating_sub(2).max(1);
                            let rows = main_area.height.saturating_sub(2).max(1);
                            pty_mgr.resize_panel(zoomed_id, cols, rows).ok();
                            if let Some(state) = self.panels.get_mut(zoomed_id) {
                                state.terminal.resize(rows, cols);
                            }
                        } else {
                            let resolved = ResolvedLayout::resolve(
                                &self.workspace.layout,
                                main_area,
                                &self.active_tab,
                            );
                            for (panel_id, rect) in &resolved.panels {
                                let cols = rect.width.saturating_sub(2).max(1);
                                let rows = rect.height.saturating_sub(2).max(1);
                                pty_mgr.resize_panel(panel_id, cols, rows).ok();
                                if let Some(state) = self.panels.get_mut(panel_id) {
                                    state.terminal.resize(rows, cols);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Periodic DB flush for recorded output (every 5 seconds)
            if self.last_db_flush.elapsed() >= Duration::from_secs(5) {
                self.flush_output_to_db();
                self.last_db_flush = Instant::now();
            }

            if self.should_quit {
                break;
            }
        }

        // Final flush of recorded output
        self.flush_output_to_db();

        // Cleanup
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    fn flush_output_to_db(&self) {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };
        for panel_id in &self.recording {
            if let Some(state) = self.panels.get(panel_id) {
                let lines: Vec<&str> = state.output_buf.lines().iter().map(|s| s.as_str()).collect();
                if !lines.is_empty() {
                    let content = lines.join("\n");
                    db.save_output(Some(&self.workspace.name), panel_id, &content).ok();
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent, pty: &mut PtyManager) -> Result<()> {
        // Global keybindings (Ctrl+...)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('n') => {
                    self.focus_next();
                    return Ok(());
                }
                KeyCode::Char('p') => {
                    self.focus_prev();
                    return Ok(());
                }
                KeyCode::Char('z') => {
                    self.toggle_zoom();
                    return Ok(());
                }
                KeyCode::Char('b') => {
                    // Toggle broadcast: cycle through groups or disable
                    self.cycle_broadcast();
                    return Ok(());
                }
                KeyCode::Char('k') => {
                    self.palette.toggle();
                    return Ok(());
                }
                KeyCode::Char('t') => {
                    // Cycle active tab (advance first found Tabs node)
                    self.cycle_tab();
                    return Ok(());
                }
                _ => {}
            }
        }

        // In broadcast mode, send to group
        if let Mode::Broadcast(ref group) = self.mode.clone() {
            let data = key_to_bytes(&key);
            if !data.is_empty() {
                match multiplexer::broadcast_to_group(pty, &self.workspace, &group, &data)? {
                    BroadcastResult::Sent(_n) => {
                        self.status_message = None;
                    }
                    BroadcastResult::Blocked(reason) => {
                        self.status_message = Some(format!("BLOCKED: {}", reason));
                    }
                    BroadcastResult::NeedsConfirmation(msg) => {
                        self.status_message = Some(format!("CONFIRM? {}", msg));
                    }
                }
            }
            return Ok(());
        }

        // Normal mode: send input to focused panel
        if let Some(panel_id) = self.focused_panel_id().map(|s| s.to_string()) {
            let data = key_to_bytes(&key);
            if !data.is_empty() {
                pty.write_to_panel(&panel_id, &data)?;
            }
        }

        Ok(())
    }

    fn handle_palette_key(&mut self, key: KeyEvent, pty: &mut PtyManager) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.palette.visible = false;
            }
            KeyCode::Enter => {
                if let Some(action) = self.palette.confirm() {
                    self.execute_palette_action(action, pty)?;
                }
            }
            KeyCode::Up => self.palette.move_up(),
            KeyCode::Down => self.palette.move_down(),
            KeyCode::Backspace => self.palette.backspace(),
            KeyCode::Char(c) => self.palette.type_char(c),
            _ => {}
        }
        Ok(())
    }

    fn execute_palette_action(&mut self, action: PaletteAction, _pty: &mut PtyManager) -> Result<()> {
        match action {
            PaletteAction::FocusPanel(id) => {
                if let Some(idx) = self.focus_order.iter().position(|s| s == &id) {
                    self.focus_index = idx;
                }
            }
            PaletteAction::ToggleZoom => self.toggle_zoom(),
            PaletteAction::ToggleBroadcast(group) => {
                if self.mode == Mode::Broadcast(group.clone()) {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::Broadcast(group);
                }
            }
            PaletteAction::ToggleRecording(panel_id) => {
                if self.recording.contains(&panel_id) {
                    self.recording.remove(&panel_id);
                    self.status_message = Some(format!("Recording OFF for {}", panel_id));
                } else {
                    self.recording.insert(panel_id.clone());
                    self.status_message = Some(format!("Recording ON for {}", panel_id));
                }
            }
            PaletteAction::SshConnect(host) => {
                // Send SSH command to focused panel
                if let Some(panel_id) = self.focused_panel_id().map(|s| s.to_string()) {
                    let cmd = format!("ssh {}\n", host);
                    _pty.write_to_panel(&panel_id, cmd.as_bytes())?;
                }
            }
            PaletteAction::Custom(cmd) => {
                if let Some(panel_id) = self.focused_panel_id().map(|s| s.to_string()) {
                    let line = format!("{}\n", cmd);
                    _pty.write_to_panel(&panel_id, line.as_bytes())?;
                }
            }
        }
        Ok(())
    }

    fn cycle_broadcast(&mut self) {
        if self.workspace.groups.is_empty() {
            self.status_message = Some("No groups defined".to_string());
            return;
        }

        match &self.mode {
            Mode::Broadcast(current) => {
                let idx = self.workspace.groups.iter().position(|g| &g.name == current);
                match idx {
                    Some(i) if i + 1 < self.workspace.groups.len() => {
                        self.mode = Mode::Broadcast(self.workspace.groups[i + 1].name.clone());
                    }
                    _ => {
                        self.mode = Mode::Normal;
                    }
                }
            }
            _ => {
                self.mode = Mode::Broadcast(self.workspace.groups[0].name.clone());
            }
        }
    }

    fn cycle_tab(&mut self) {
        // Find all Tabs nodes and cycle their active tab
        fn find_tabs_ids(node: &tp_core::workspace::LayoutNode) -> Vec<(String, usize)> {
            match node {
                tp_core::workspace::LayoutNode::Tabs { children, labels } => {
                    let key = labels.first().cloned().unwrap_or_else(|| "tabs".to_string());
                    vec![(key, children.len())]
                }
                tp_core::workspace::LayoutNode::Hsplit { children, .. }
                | tp_core::workspace::LayoutNode::Vsplit { children, .. } => {
                    children.iter().flat_map(find_tabs_ids).collect()
                }
                _ => vec![],
            }
        }
        let tabs = find_tabs_ids(&self.workspace.layout);
        for (key, count) in tabs {
            if count > 0 {
                let current = self.active_tab.get(&key).copied().unwrap_or(0);
                let next = (current + 1) % count;
                self.active_tab.insert(key, next);
            }
        }
        // Update focus_order based on now-visible panels
        self.focus_order = self.workspace.layout.panel_ids().iter().map(|s| s.to_string()).collect();
    }

    fn build_status_line(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();

        // Mode indicator
        let mode_str = match &self.mode {
            Mode::Normal => " NORMAL ".to_string(),
            Mode::Broadcast(g) => format!(" BC:{} ", g),
            Mode::Palette => " PALETTE ".to_string(),
            Mode::Zoom => " ZOOM ".to_string(),
        };
        spans.push(Span::styled(
            mode_str,
            Style::default().bg(Color::Blue).fg(Color::White),
        ));

        // Focused panel
        if let Some(id) = self.focused_panel_id() {
            let name = self.workspace
                .panel(id)
                .map(|p| p.name.as_str())
                .unwrap_or(id);
            spans.push(Span::raw(format!(" [{}] ", name)));
        }

        // Workspace name
        spans.push(Span::styled(
            format!(" {} ", self.workspace.name),
            Style::default().fg(Color::DarkGray),
        ));

        // Status message
        if let Some(ref msg) = self.status_message {
            spans.push(Span::styled(
                format!(" {} ", msg),
                Style::default().fg(Color::Yellow),
            ));
        }

        // Recording indicator
        if let Some(id) = self.focused_panel_id() {
            if self.recording.contains(id) {
                spans.push(Span::styled(
                    " REC ",
                    Style::default().bg(Color::Red).fg(Color::White),
                ));
            }
        }

        // Keybind hints
        spans.push(Span::styled(
            " C-q:quit C-n/p:focus C-z:zoom C-b:broadcast C-t:tab C-k:palette ",
            Style::default().fg(Color::DarkGray),
        ));

        spans
    }
}

/// Convert a crossterm KeyEvent to bytes to send to the PTY.
fn key_to_bytes(key: &KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                // Ctrl+A..Z → 0x01..0x1A
                let byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                if byte <= 26 {
                    return vec![byte];
                }
            }
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => vec![],
        },
        _ => vec![],
    }
}

fn build_palette_items(ws: &Workspace) -> Vec<PaletteItem> {
    let mut items = Vec::new();

    // Focus panel commands
    for panel in &ws.panels {
        items.push(PaletteItem {
            label: format!("Focus: {}", panel.name),
            description: format!("Switch focus to panel {}", panel.id),
            action: PaletteAction::FocusPanel(panel.id.clone()),
        });
    }

    // Zoom toggle
    items.push(PaletteItem {
        label: "Toggle Zoom".to_string(),
        description: "Toggle full-page zoom for focused panel".to_string(),
        action: PaletteAction::ToggleZoom,
    });

    // Broadcast group toggles
    for group in &ws.groups {
        items.push(PaletteItem {
            label: format!("Broadcast: {}", group.name),
            description: format!("Toggle broadcast to group {}", group.name),
            action: PaletteAction::ToggleBroadcast(group.name.clone()),
        });
    }

    // Recording toggles
    for panel in &ws.panels {
        items.push(PaletteItem {
            label: format!("Record: {}", panel.name),
            description: format!("Toggle output recording for panel {}", panel.id),
            action: PaletteAction::ToggleRecording(panel.id.clone()),
        });
    }

    // SSH hosts from ~/.ssh/config
    if let Ok(hosts) = ssh::parse_default_ssh_config() {
        for host in hosts {
            let display = if let Some(ref user) = host.user {
                format!("{}@{}", user, host.name)
            } else {
                host.name.clone()
            };
            items.push(PaletteItem {
                label: format!("SSH: {}", display),
                description: format!(
                    "Connect to {} ({})",
                    host.name,
                    host.hostname.as_deref().unwrap_or(&host.name)
                ),
                action: PaletteAction::SshConnect(host.name.clone()),
            });
        }
    }

    items
}

fn color_from_name(name: &str) -> Color {
    match name.to_lowercase().as_str() {
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        _ => Color::Red,
    }
}
