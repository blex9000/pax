//! # PTY Terminal Backend (Cross-platform fallback)
//!
//! Fallback terminal backend for platforms where VTE4 is not available (macOS).
//! Uses `portable-pty` for PTY management, `alacritty_terminal` for terminal
//! emulation, and a GTK4 `DrawingArea` renderer for viewport painting.
//!
//! ## Current limitations (vs VTE backend)
//! - No hyperlink activation
//! - No OSC 7 directory tracking
//! - No mouse reporting
//! - No advanced IME/text shaping
//!
//! ## Why this exists
//! VTE4 is a Linux-only library (requires X11/Wayland). macOS can run GTK4 apps
//! via Homebrew, but VTE4 is not practically available. This backend provides a
//! much closer terminal experience than the old `vt100 + TextView` fallback
//! while keeping the same PTY and panel architecture.

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Processor, Rgb};
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use pangocairo::functions as pangocairo;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::cell::RefCell;
use std::fmt;
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;
const DEFAULT_SCROLLBACK: usize = 10_000;
const SCROLL_MULTIPLIER: f64 = 3.0;

pub struct TerminalInner {
    pub drawing_area: gtk4::DrawingArea,
    pub widget: gtk4::Widget,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
}

impl fmt::Debug for TerminalInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalInner")
            .field("type", &"pty-fallback")
            .finish()
    }
}

impl TerminalInner {
    pub fn new(
        shell: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        _workspace_dir: Option<&str>,
    ) -> Self {
        let drawing_area = gtk4::DrawingArea::new();
        drawing_area.set_focusable(true);
        drawing_area.set_vexpand(true);
        drawing_area.set_hexpand(true);
        drawing_area.add_css_class("terminal-fallback");

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Never);
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        scrolled.set_propagate_natural_height(false);
        scrolled.set_propagate_natural_width(false);
        scrolled.set_child(Some(&drawing_area));

        let widget = scrolled.clone().upcast::<gtk4::Widget>();

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY");

        let mut cmd = CommandBuilder::new(shell);
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");

        let _child = pair
            .slave
            .spawn_command(cmd)
            .expect("Failed to spawn shell");
        drop(pair.slave);

        let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));
        let writer = {
            let guard = master.lock().expect("PTY master lock poisoned");
            guard.take_writer().expect("Failed to take writer")
        };
        let mut reader = {
            let guard = master.lock().expect("PTY master lock poisoned");
            guard.try_clone_reader().expect("Failed to clone reader")
        };

        let writer = Arc::new(Mutex::new(writer));
        let input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>> =
            Rc::new(RefCell::new(None));

        let (ui_tx, ui_rx) = mpsc::channel::<TerminalUiEvent>();
        let window_size = Arc::new(Mutex::new(GridSize::default().window_size()));
        let event_proxy = TerminalEventProxy {
            writer: writer.clone(),
            ui_tx: ui_tx.clone(),
            window_size: window_size.clone(),
        };
        let term_state = Arc::new(Mutex::new(TermState::new(event_proxy)));

        {
            let term_state = term_state.clone();
            let ui_tx = ui_tx.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Ok(mut state) = term_state.lock() {
                                let TermState { term, parser } = &mut *state;
                                parser.advance(term, &buf[..n]);
                            }
                            let _ = ui_tx.send(TerminalUiEvent::Render);
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        {
            let drawing_area = drawing_area.clone();
            let writer = writer.clone();
            glib::timeout_add_local(Duration::from_millis(16), move || {
                loop {
                    match ui_rx.try_recv() {
                        Ok(TerminalUiEvent::Render) => drawing_area.queue_draw(),
                        Ok(TerminalUiEvent::ClipboardStore(text)) => {
                            drawing_area.clipboard().set_text(&text);
                        }
                        Ok(TerminalUiEvent::ClipboardLoad(formatter)) => {
                            let writer = writer.clone();
                            drawing_area.clipboard().read_text_async(
                                None::<&gtk4::gio::Cancellable>,
                                move |result| {
                                    if let Ok(Some(text)) = result {
                                        let response = formatter(text.as_str());
                                        let _ = write_bytes(&writer, response.as_bytes());
                                    }
                                },
                            );
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        {
            let term_state = term_state.clone();
            drawing_area.set_draw_func(move |area, cr, width, height| {
                draw_terminal(area, cr, width, height, &term_state);
            });
        }

        {
            let master = master.clone();
            let term_state = term_state.clone();
            let window_size = window_size.clone();
            let last_grid = Rc::new(RefCell::new(GridSize::default()));
            let last_grid_resize = last_grid.clone();
            let drawing_area_resize = drawing_area.clone();
            drawing_area.connect_resize(move |area, width, height| {
                let Some(metrics) = measure_cell_metrics(area) else {
                    return;
                };
                let Some(size) = grid_size_for_area(width, height, metrics.width, metrics.height)
                else {
                    return;
                };
                if *last_grid_resize.borrow() == size {
                    return;
                }
                *last_grid_resize.borrow_mut() = size;
                if let Ok(mut ws) = window_size.lock() {
                    *ws = size.window_size();
                }
                if let Ok(master) = master.lock() {
                    let _ = master.resize(size.pty_size());
                }
                if let Ok(mut state) = term_state.lock() {
                    state.term.resize(size);
                }
                drawing_area_resize.queue_draw();
            });
        }

        {
            let drawing_area_focus = drawing_area.clone();
            let click = gtk4::GestureClick::new();
            click.set_button(1);
            click.connect_pressed(move |_, _, _, _| {
                drawing_area_focus.grab_focus();
            });
            drawing_area.add_controller(click);
        }

        {
            let selection_anchor = Rc::new(RefCell::new(None::<DragSelectionAnchor>));
            let term_state = term_state.clone();
            let drag_area = drawing_area.clone();
            let drag = gtk4::GestureDrag::new();
            drag.set_button(1);
            {
                let selection_anchor = selection_anchor.clone();
                let term_state = term_state.clone();
                let drag_area = drag_area.clone();
                drag.connect_drag_begin(move |_gesture, x, y| {
                    drag_area.grab_focus();
                    let Some(metrics) = measure_cell_metrics(&drag_area) else {
                        return;
                    };
                    let Ok(mut state) = term_state.lock() else {
                        return;
                    };
                    let Some(point) = point_from_coords(&state.term, metrics, x, y) else {
                        return;
                    };
                    *selection_anchor.borrow_mut() = Some(DragSelectionAnchor { point, x, y });
                    state.term.selection = Some(simple_selection(point, point));
                    drag_area.queue_draw();
                });
            }
            {
                let selection_anchor = selection_anchor.clone();
                let term_state = term_state.clone();
                let drag_area = drag_area.clone();
                drag.connect_drag_update(move |_gesture, dx, dy| {
                    let Some(anchor) = *selection_anchor.borrow() else {
                        return;
                    };
                    let Some(metrics) = measure_cell_metrics(&drag_area) else {
                        return;
                    };
                    let Ok(mut state) = term_state.lock() else {
                        return;
                    };
                    let Some(point) =
                        point_from_coords(&state.term, metrics, anchor.x + dx, anchor.y + dy)
                    else {
                        return;
                    };
                    state.term.selection = Some(simple_selection(anchor.point, point));
                    drag_area.queue_draw();
                });
            }
            {
                let selection_anchor = selection_anchor.clone();
                drag.connect_drag_end(move |_, _, _| {
                    *selection_anchor.borrow_mut() = None;
                });
            }
            drawing_area.add_controller(drag);
        }

        {
            let term_state = term_state.clone();
            let scroll_area = drawing_area.clone();
            let scroll = gtk4::EventControllerScroll::new(
                gtk4::EventControllerScrollFlags::VERTICAL
                    | gtk4::EventControllerScrollFlags::DISCRETE,
            );
            scroll.connect_scroll(move |_, _dx, dy| {
                let step = normalize_scroll_delta(dy);
                if step == 0 {
                    return glib::Propagation::Proceed;
                }
                if let Ok(mut state) = term_state.lock() {
                    state.term.scroll_display(Scroll::Delta(step));
                }
                scroll_area.queue_draw();
                glib::Propagation::Stop
            });
            drawing_area.add_controller(scroll);
        }

        {
            let writer = writer.clone();
            let input_cb = input_cb.clone();
            let term_state = term_state.clone();
            let key_controller = gtk4::EventControllerKey::new();
            key_controller.connect_key_pressed(move |ctrl, key, _code, modifiers| {
                let widget = ctrl.widget().and_downcast::<gtk4::DrawingArea>();
                if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                    && modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                {
                    if matches!(key, gdk::Key::c | gdk::Key::C) {
                        if let Some(widget) = widget {
                            if let Some(text) = selection_text(&term_state) {
                                widget.clipboard().set_text(&text);
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                    if matches!(key, gdk::Key::v | gdk::Key::V) {
                        if let Some(widget) = widget {
                            let writer = writer.clone();
                            let input_cb = input_cb.clone();
                            widget.clipboard().read_text_async(
                                None::<&gtk4::gio::Cancellable>,
                                move |result| {
                                    if let Ok(Some(text)) = result {
                                        let _ =
                                            send_user_input(&writer, &input_cb, text.as_bytes());
                                    }
                                },
                            );
                        }
                        return glib::Propagation::Stop;
                    }
                }

                let Some(bytes) = super::input::encode_key_input(key, modifiers) else {
                    return glib::Propagation::Proceed;
                };
                let _ = send_user_input(&writer, &input_cb, &bytes);
                glib::Propagation::Stop
            });
            drawing_area.add_controller(key_controller);
        }

        setup_context_menu(&drawing_area, &term_state, &writer, &input_cb);

        Self {
            drawing_area,
            widget,
            writer,
            input_cb,
        }
    }

    /// Send commands to the running shell.
    pub fn send_commands(&self, commands: &[String]) {
        if let Ok(mut w) = self.writer.lock() {
            for cmd in commands {
                let line = format!("{}\n", cmd);
                let _ = w.write_all(line.as_bytes());
                let _ = w.flush();
            }
        }
    }

    /// Queue raw text (same as send_commands for PTY backend since there's
    /// no spawn callback — commands go directly to the shell).
    pub fn queue_raw(&self, text: &str) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
    }

    pub fn write_input(&self, data: &[u8]) -> bool {
        write_bytes(&self.writer, data)
    }

    pub fn set_input_callback(&self, callback: Option<crate::panels::PanelInputCallback>) {
        *self.input_cb.borrow_mut() = callback;
    }

    pub fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    pub fn grab_focus(&self) {
        self.drawing_area.grab_focus();
    }
}

struct TermState {
    term: Term<TerminalEventProxy>,
    parser: Processor,
}

impl TermState {
    fn new(listener: TerminalEventProxy) -> Self {
        let size = GridSize::default();
        let config = TermConfig {
            scrolling_history: DEFAULT_SCROLLBACK,
            ..TermConfig::default()
        };
        Self {
            term: Term::new(config, &size, listener),
            parser: Processor::default(),
        }
    }
}

#[derive(Clone)]
struct TerminalEventProxy {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    ui_tx: mpsc::Sender<TerminalUiEvent>,
    window_size: Arc<Mutex<WindowSize>>,
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::ClipboardStore(_, text) => {
                let _ = self.ui_tx.send(TerminalUiEvent::ClipboardStore(text));
            }
            Event::ClipboardLoad(_, formatter) => {
                let _ = self.ui_tx.send(TerminalUiEvent::ClipboardLoad(formatter));
            }
            Event::PtyWrite(text) => {
                let _ = write_bytes(&self.writer, text.as_bytes());
            }
            Event::TextAreaSizeRequest(formatter) => {
                let window_size = self
                    .window_size
                    .lock()
                    .map(|guard| *guard)
                    .unwrap_or(GridSize::default().window_size());
                let response = formatter(window_size);
                let _ = write_bytes(&self.writer, response.as_bytes());
            }
            Event::Wakeup | Event::CursorBlinkingChange | Event::MouseCursorDirty => {
                let _ = self.ui_tx.send(TerminalUiEvent::Render);
            }
            Event::Title(_)
            | Event::ResetTitle
            | Event::ColorRequest(_, _)
            | Event::Bell
            | Event::Exit
            | Event::ChildExit(_) => {}
        }
    }
}

enum TerminalUiEvent {
    Render,
    ClipboardStore(String),
    ClipboardLoad(Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
}

#[derive(Clone, Copy)]
struct DragSelectionAnchor {
    point: Point,
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellMetrics {
    width: i32,
    height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridSize {
    rows: u16,
    cols: u16,
    cell_width: u16,
    cell_height: u16,
}

impl Default for GridSize {
    fn default() -> Self {
        Self {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            cell_width: 0,
            cell_height: 0,
        }
    }
}

impl GridSize {
    fn pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: self.cell_width,
            pixel_height: self.cell_height,
        }
    }

    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.cols,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        self.cols as usize
    }
}

#[derive(Clone, Copy)]
struct TerminalPalette {
    background: Rgb,
    foreground: Rgb,
    cursor_bg: Rgb,
    cursor_fg: Rgb,
    selection_bg: Rgb,
    selection_fg: Rgb,
    ansi: [Rgb; 16],
}

impl TerminalPalette {
    fn current() -> Self {
        terminal_palette_for(
            crate::theme::current_theme(),
            libadwaita::StyleManager::default().is_dark(),
        )
    }
}

fn terminal_font_spec() -> String {
    resolve_terminal_font_spec(
        std::env::var("PAX_TERMINAL_FONT").ok().as_deref(),
        cfg!(target_os = "macos"),
    )
}

fn resolve_terminal_font_spec(override_font: Option<&str>, is_macos: bool) -> String {
    if let Some(font) = override_font.map(str::trim).filter(|font| !font.is_empty()) {
        return font.to_string();
    }

    if is_macos {
        "Menlo 13".to_string()
    } else {
        "Monospace 11".to_string()
    }
}

fn terminal_palette_for(theme: crate::theme::Theme, system_dark: bool) -> TerminalPalette {
    match theme {
        crate::theme::Theme::System => {
            if system_dark {
                make_terminal_palette(
                    0x111827,
                    0xe5e7eb,
                    0x374151,
                    0xf9fafb,
                    [
                        0x1f2937, 0xf87171, 0x34d399, 0xfbbf24, 0x60a5fa, 0xc084fc, 0x22d3ee,
                        0xe5e7eb, 0x4b5563, 0xfca5a5, 0x6ee7b7, 0xfcd34d, 0x93c5fd, 0xd8b4fe,
                        0x67e8f9, 0xf9fafb,
                    ],
                )
            } else {
                make_terminal_palette(
                    0xffffff,
                    0x1f2937,
                    0xdbeafe,
                    0x111827,
                    [
                        0x111827, 0xdc2626, 0x059669, 0xd97706, 0x2563eb, 0x9333ea, 0x0891b2,
                        0xe5e7eb, 0x6b7280, 0xef4444, 0x10b981, 0xf59e0b, 0x3b82f6, 0xa855f7,
                        0x06b6d4, 0xf9fafb,
                    ],
                )
            }
        }
        crate::theme::Theme::CatppuccinMocha => make_terminal_palette(
            0x1e1e2e,
            0xcdd6f4,
            0x45475a,
            0xf5e0dc,
            [
                0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
                0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
            ],
        ),
        crate::theme::Theme::CatppuccinLatte => make_terminal_palette(
            0xeff1f5,
            0x4c4f69,
            0xbcc0cc,
            0x4c4f69,
            [
                0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
                0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xdc8a78,
            ],
        ),
        crate::theme::Theme::Dracula => make_terminal_palette(
            0x282a36,
            0xf8f8f2,
            0x44475a,
            0xf8f8f2,
            [
                0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
            ],
        ),
        crate::theme::Theme::Nord => make_terminal_palette(
            0x2e3440,
            0xeceff4,
            0x434c5e,
            0xeceff4,
            [
                0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
            ],
        ),
    }
}

fn make_terminal_palette(
    background: u32,
    foreground: u32,
    selection_bg: u32,
    selection_fg: u32,
    ansi: [u32; 16],
) -> TerminalPalette {
    TerminalPalette {
        background: rgb(background),
        foreground: rgb(foreground),
        cursor_bg: rgb(foreground),
        cursor_fg: rgb(background),
        selection_bg: rgb(selection_bg),
        selection_fg: rgb(selection_fg),
        ansi: ansi.map(rgb),
    }
}

fn rgb(hex: u32) -> Rgb {
    Rgb {
        r: ((hex >> 16) & 0xff) as u8,
        g: ((hex >> 8) & 0xff) as u8,
        b: (hex & 0xff) as u8,
    }
}

fn setup_context_menu(
    drawing_area: &gtk4::DrawingArea,
    term_state: &Arc<Mutex<TermState>>,
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    input_cb: &Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
) {
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let area = drawing_area.clone();
    let state = term_state.clone();
    let writer = writer.clone();
    let input_cb = input_cb.clone();
    gesture.connect_pressed(move |_gesture, _n, x, y| {
        let popover = gtk4::PopoverMenu::from_model(None::<&gtk4::gio::MenuModel>);

        let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        menu_box.set_margin_top(4);
        menu_box.set_margin_bottom(4);
        menu_box.set_margin_start(4);
        menu_box.set_margin_end(4);

        let copy_text = selection_text(&state);

        let copy_btn = gtk4::Button::new();
        copy_btn.add_css_class("flat");
        copy_btn.set_sensitive(copy_text.is_some());
        let copy_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        copy_box.append(&gtk4::Image::from_icon_name("edit-copy-symbolic"));
        let copy_lbl = gtk4::Label::new(Some("Copy"));
        copy_lbl.set_hexpand(true);
        copy_lbl.set_halign(gtk4::Align::Start);
        copy_box.append(&copy_lbl);
        let copy_hint = gtk4::Label::new(Some("Ctrl+Shift+C"));
        copy_hint.add_css_class("dim-label");
        copy_box.append(&copy_hint);
        copy_btn.set_child(Some(&copy_box));
        let area_copy = area.clone();
        let popover_copy = popover.clone();
        copy_btn.connect_clicked(move |_| {
            if let Some(text) = copy_text.as_deref() {
                area_copy.clipboard().set_text(text);
            }
            popover_copy.popdown();
        });
        menu_box.append(&copy_btn);

        let paste_btn = gtk4::Button::new();
        paste_btn.add_css_class("flat");
        let paste_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        paste_box.append(&gtk4::Image::from_icon_name("edit-paste-symbolic"));
        let paste_lbl = gtk4::Label::new(Some("Paste"));
        paste_lbl.set_hexpand(true);
        paste_lbl.set_halign(gtk4::Align::Start);
        paste_box.append(&paste_lbl);
        let paste_hint = gtk4::Label::new(Some("Ctrl+Shift+V"));
        paste_hint.add_css_class("dim-label");
        paste_box.append(&paste_hint);
        paste_btn.set_child(Some(&paste_box));
        let area_paste = area.clone();
        let writer_paste = writer.clone();
        let input_cb_paste = input_cb.clone();
        let popover_paste = popover.clone();
        paste_btn.connect_clicked(move |_| {
            let writer_paste = writer_paste.clone();
            let input_cb_paste = input_cb_paste.clone();
            area_paste.clipboard().read_text_async(
                None::<&gtk4::gio::Cancellable>,
                move |result| {
                    if let Ok(Some(text)) = result {
                        let _ = send_user_input(&writer_paste, &input_cb_paste, text.as_bytes());
                    }
                },
            );
            popover_paste.popdown();
        });
        menu_box.append(&paste_btn);

        popover.set_child(Some(&menu_box));
        popover.set_parent(&area);
        popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    drawing_area.add_controller(gesture);
}

fn draw_terminal(
    area: &gtk4::DrawingArea,
    cr: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    term_state: &Arc<Mutex<TermState>>,
) {
    let Some(metrics) = measure_cell_metrics(area) else {
        return;
    };
    let palette = TerminalPalette::current();
    paint_rgb(cr, palette.background);
    cr.rectangle(0.0, 0.0, width as f64, height as f64);
    let _ = cr.fill();

    let Ok(state) = term_state.lock() else {
        return;
    };
    let renderable = state.term.renderable_content();
    let cursor = renderable.cursor;
    let selection = renderable.selection;
    let display_offset = renderable.display_offset as i32;
    let colors = renderable.colors;

    let layout = pangocairo::create_layout(cr);
    layout.set_font_description(Some(&terminal_font_description()));

    for indexed in renderable.display_iter {
        let row = indexed.point.line.0 + display_offset;
        if row < 0 {
            continue;
        }
        let row = row as i32;
        if row >= state.term.screen_lines() as i32 {
            continue;
        }

        if indexed
            .cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }

        let col = indexed.point.column.0 as i32;
        let x = col * metrics.width;
        let y = row * metrics.height;
        let cell_width = if indexed.cell.flags.contains(Flags::WIDE_CHAR) {
            metrics.width * 2
        } else {
            metrics.width
        };

        let selected = selection
            .map(|selection| selection.contains_cell(&indexed, cursor.point, cursor.shape))
            .unwrap_or(false);

        let (mut fg, mut bg) = resolve_cell_colors(indexed.cell, &palette, colors);
        if selected {
            fg = palette.selection_fg;
            bg = palette.selection_bg;
        }

        if bg != palette.background || selected {
            paint_rgb(cr, bg);
            cr.rectangle(x as f64, y as f64, cell_width as f64, metrics.height as f64);
            let _ = cr.fill();
        }

        if !indexed.cell.flags.contains(Flags::HIDDEN) {
            let text = cell_text(indexed.cell);
            if !text.is_empty() && text != " " {
                paint_rgb(cr, fg);
                layout.set_text(&text);
                cr.move_to(x as f64, y as f64);
                pangocairo::show_layout(cr, &layout);
            }
        }

        if indexed.cell.flags.intersects(Flags::ALL_UNDERLINES) {
            paint_rgb(cr, fg);
            cr.set_line_width(1.0);
            cr.move_to(x as f64, (y + metrics.height - 2) as f64);
            cr.line_to((x + cell_width) as f64, (y + metrics.height - 2) as f64);
            let _ = cr.stroke();
        }
        if indexed.cell.flags.contains(Flags::STRIKEOUT) {
            paint_rgb(cr, fg);
            cr.set_line_width(1.0);
            cr.move_to(x as f64, (y + metrics.height / 2) as f64);
            cr.line_to((x + cell_width) as f64, (y + metrics.height / 2) as f64);
            let _ = cr.stroke();
        }
    }

    draw_cursor(cr, &state.term, metrics, palette, cursor, display_offset);
}

fn draw_cursor(
    cr: &gtk4::cairo::Context,
    term: &Term<TerminalEventProxy>,
    metrics: CellMetrics,
    palette: TerminalPalette,
    cursor: alacritty_terminal::term::RenderableCursor,
    display_offset: i32,
) {
    if cursor.shape == CursorShape::Hidden {
        return;
    }

    let row = cursor.point.line.0 + display_offset;
    if row < 0 || row >= term.screen_lines() as i32 {
        return;
    }

    let col = cursor.point.column.0 as i32;
    let x = col * metrics.width;
    let y = row * metrics.height;
    let cell = &term.grid()[cursor.point];
    let width = if cell.flags.contains(Flags::WIDE_CHAR) {
        metrics.width * 2
    } else {
        metrics.width
    };

    match cursor.shape {
        CursorShape::Block => {
            paint_rgb(cr, palette.cursor_bg);
            cr.rectangle(x as f64, y as f64, width as f64, metrics.height as f64);
            let _ = cr.fill();

            let text = cell_text(cell);
            if !text.is_empty() && text != " " && !cell.flags.contains(Flags::HIDDEN) {
                let layout = pangocairo::create_layout(cr);
                layout.set_font_description(Some(&terminal_font_description()));
                layout.set_text(&text);
                paint_rgb(cr, palette.cursor_fg);
                cr.move_to(x as f64, y as f64);
                pangocairo::show_layout(cr, &layout);
            }
        }
        CursorShape::Underline => {
            paint_rgb(cr, palette.cursor_bg);
            cr.set_line_width(2.0);
            cr.move_to(x as f64, (y + metrics.height - 1) as f64);
            cr.line_to((x + width) as f64, (y + metrics.height - 1) as f64);
            let _ = cr.stroke();
        }
        CursorShape::Beam => {
            paint_rgb(cr, palette.cursor_bg);
            cr.set_line_width(2.0);
            cr.move_to(x as f64 + 1.0, y as f64);
            cr.line_to(x as f64 + 1.0, (y + metrics.height) as f64);
            let _ = cr.stroke();
        }
        CursorShape::HollowBlock => {
            paint_rgb(cr, palette.cursor_bg);
            cr.set_line_width(1.0);
            cr.rectangle(
                x as f64 + 0.5,
                y as f64 + 0.5,
                (width - 1).max(1) as f64,
                (metrics.height - 1).max(1) as f64,
            );
            let _ = cr.stroke();
        }
        CursorShape::Hidden => {}
    }
}

fn measure_cell_metrics<W: IsA<gtk4::Widget>>(widget: &W) -> Option<CellMetrics> {
    let layout = widget.create_pango_layout(Some("W"));
    layout.set_font_description(Some(&terminal_font_description()));
    let (width, height) = layout.pixel_size();
    (width > 0 && height > 0).then_some(CellMetrics { width, height })
}

fn terminal_font_description() -> gtk4::pango::FontDescription {
    gtk4::pango::FontDescription::from_string(&terminal_font_spec())
}

fn point_from_coords<T: EventListener>(
    term: &Term<T>,
    metrics: CellMetrics,
    x: f64,
    y: f64,
) -> Option<Point> {
    if metrics.width <= 0 || metrics.height <= 0 {
        return None;
    }
    if term.columns() == 0 || term.screen_lines() == 0 {
        return None;
    }

    let col = ((x / metrics.width as f64).floor() as isize)
        .clamp(0, term.columns().saturating_sub(1) as isize) as usize;
    let row = ((y / metrics.height as f64).floor() as isize)
        .clamp(0, term.screen_lines().saturating_sub(1) as isize) as i32;

    Some(Point::new(
        Line(row - term.grid().display_offset() as i32),
        Column(col),
    ))
}

fn simple_selection(start: Point, end: Point) -> Selection {
    let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
    selection.update(end, Side::Right);
    selection.include_all();
    selection
}

fn selection_text(term_state: &Arc<Mutex<TermState>>) -> Option<String> {
    term_state
        .lock()
        .ok()
        .and_then(|state| state.term.selection_to_string())
        .filter(|text| !text.is_empty())
}

fn resolve_cell_colors(
    cell: &Cell,
    palette: &TerminalPalette,
    colors: &alacritty_terminal::term::color::Colors,
) -> (Rgb, Rgb) {
    let bold = cell
        .flags
        .intersects(Flags::BOLD | Flags::BOLD_ITALIC | Flags::DIM_BOLD);
    let dim = cell.flags.intersects(Flags::DIM | Flags::DIM_BOLD);

    let mut fg = resolve_ansi_color(cell.fg, palette, colors, bold, dim);
    let mut bg = resolve_ansi_color(cell.bg, palette, colors, false, false);

    if cell.flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    (fg, bg)
}

fn resolve_ansi_color(
    color: AnsiColor,
    palette: &TerminalPalette,
    colors: &alacritty_terminal::term::color::Colors,
    bold: bool,
    dim: bool,
) -> Rgb {
    match color {
        AnsiColor::Spec(rgb) => rgb,
        AnsiColor::Indexed(index) => indexed_rgb(index, palette),
        AnsiColor::Named(named) => resolve_named_color(named, palette, colors, bold, dim),
    }
}

fn resolve_named_color(
    mut named: NamedColor,
    palette: &TerminalPalette,
    colors: &alacritty_terminal::term::color::Colors,
    bold: bool,
    dim: bool,
) -> Rgb {
    if bold {
        named = named.to_bright();
    } else if dim {
        named = named.to_dim();
    }

    if let Some(rgb) = colors[named] {
        return rgb;
    }

    match named {
        NamedColor::Foreground | NamedColor::BrightForeground => palette.foreground,
        NamedColor::Background => palette.background,
        NamedColor::Cursor => palette.cursor_bg,
        NamedColor::DimForeground => dim_rgb(palette.foreground),
        NamedColor::Black => palette.ansi[0],
        NamedColor::Red => palette.ansi[1],
        NamedColor::Green => palette.ansi[2],
        NamedColor::Yellow => palette.ansi[3],
        NamedColor::Blue => palette.ansi[4],
        NamedColor::Magenta => palette.ansi[5],
        NamedColor::Cyan => palette.ansi[6],
        NamedColor::White => palette.ansi[7],
        NamedColor::BrightBlack => palette.ansi[8],
        NamedColor::BrightRed => palette.ansi[9],
        NamedColor::BrightGreen => palette.ansi[10],
        NamedColor::BrightYellow => palette.ansi[11],
        NamedColor::BrightBlue => palette.ansi[12],
        NamedColor::BrightMagenta => palette.ansi[13],
        NamedColor::BrightCyan => palette.ansi[14],
        NamedColor::BrightWhite => palette.ansi[15],
        NamedColor::DimBlack => dim_rgb(palette.ansi[0]),
        NamedColor::DimRed => dim_rgb(palette.ansi[1]),
        NamedColor::DimGreen => dim_rgb(palette.ansi[2]),
        NamedColor::DimYellow => dim_rgb(palette.ansi[3]),
        NamedColor::DimBlue => dim_rgb(palette.ansi[4]),
        NamedColor::DimMagenta => dim_rgb(palette.ansi[5]),
        NamedColor::DimCyan => dim_rgb(palette.ansi[6]),
        NamedColor::DimWhite => dim_rgb(palette.ansi[7]),
    }
}

fn indexed_rgb(index: u8, palette: &TerminalPalette) -> Rgb {
    match index {
        0..=15 => palette.ansi[index as usize],
        16..=231 => {
            let index = index - 16;
            let r = index / 36;
            let g = (index / 6) % 6;
            let b = index % 6;
            Rgb {
                r: cube_level(r),
                g: cube_level(g),
                b: cube_level(b),
            }
        }
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            Rgb {
                r: level,
                g: level,
                b: level,
            }
        }
    }
}

fn cube_level(level: u8) -> u8 {
    match level {
        0 => 0,
        1 => 95,
        2 => 135,
        3 => 175,
        4 => 215,
        _ => 255,
    }
}

fn dim_rgb(rgb: Rgb) -> Rgb {
    Rgb {
        r: ((rgb.r as f32) * 0.66) as u8,
        g: ((rgb.g as f32) * 0.66) as u8,
        b: ((rgb.b as f32) * 0.66) as u8,
    }
}

fn cell_text(cell: &Cell) -> String {
    let mut text = String::new();
    if !cell.flags.contains(Flags::HIDDEN) && cell.c != ' ' {
        text.push(cell.c);
        if let Some(extra) = cell.zerowidth() {
            for c in extra {
                text.push(*c);
            }
        }
    } else if cell.c == ' ' {
        text.push(' ');
    }
    text
}

fn paint_rgb(cr: &gtk4::cairo::Context, rgb: Rgb) {
    cr.set_source_rgb(
        rgb.r as f64 / 255.0,
        rgb.g as f64 / 255.0,
        rgb.b as f64 / 255.0,
    );
}

fn normalize_scroll_delta(dy: f64) -> i32 {
    if dy == 0.0 {
        return 0;
    }
    (-dy * SCROLL_MULTIPLIER).round() as i32
}

fn write_bytes(writer: &Arc<Mutex<Box<dyn Write + Send>>>, data: &[u8]) -> bool {
    if let Ok(mut writer) = writer.lock() {
        writer.write_all(data).is_ok() && writer.flush().is_ok()
    } else {
        false
    }
}

fn send_user_input(
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    input_cb: &Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
    data: &[u8],
) -> bool {
    let ok = write_bytes(writer, data);
    if ok {
        if let Ok(borrowed) = input_cb.try_borrow() {
            if let Some(ref cb) = *borrowed {
                cb(data);
            }
        }
    }
    ok
}

fn grid_size_for_area(
    width: i32,
    height: i32,
    cell_width: i32,
    cell_height: i32,
) -> Option<GridSize> {
    if width <= 0 || height <= 0 || cell_width <= 0 || cell_height <= 0 {
        return None;
    }

    Some(GridSize {
        rows: (height / cell_height).max(1) as u16,
        cols: (width / cell_width).max(1) as u16,
        cell_width: cell_width.min(u16::MAX as i32) as u16,
        cell_height: cell_height.min(u16::MAX as i32) as u16,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[derive(Clone, Default)]
    struct SharedWriter {
        written: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written
                .lock()
                .expect("shared writer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn grid_dimensions_scale_with_available_area() {
        let size = grid_size_for_area(800, 480, 10, 20).expect("size");
        assert_eq!(size.cols, 80);
        assert_eq!(size.rows, 24);
        assert_eq!(size.cell_width, 10);
        assert_eq!(size.cell_height, 20);
    }

    #[test]
    fn grid_dimensions_clamp_to_minimum_cell() {
        let size = grid_size_for_area(5, 5, 10, 20).expect("size");
        assert_eq!(size.cols, 1);
        assert_eq!(size.rows, 1);
    }

    #[test]
    fn grid_dimensions_reject_invalid_metrics() {
        assert!(grid_size_for_area(0, 100, 10, 20).is_none());
        assert!(grid_size_for_area(100, 100, 0, 20).is_none());
    }

    #[test]
    fn indexed_rgb_uses_standard_color_cube() {
        let palette = terminal_palette_for(crate::theme::Theme::Dracula, true);
        let rgb = indexed_rgb(21, &palette);
        assert_eq!(rgb, Rgb { r: 0, g: 0, b: 255 });
    }

    #[test]
    fn terminal_font_spec_prefers_explicit_override() {
        assert_eq!(
            resolve_terminal_font_spec(Some("JetBrains Mono 14"), true),
            "JetBrains Mono 14"
        );
    }

    #[test]
    fn terminal_font_spec_uses_platform_defaults() {
        assert_eq!(resolve_terminal_font_spec(None, true), "Menlo 13");
        assert_eq!(resolve_terminal_font_spec(None, false), "Monospace 11");
    }

    #[test]
    fn terminal_palette_matches_dracula_theme() {
        let palette = terminal_palette_for(crate::theme::Theme::Dracula, true);
        assert_eq!(palette.background, rgb(0x282a36));
        assert_eq!(palette.foreground, rgb(0xf8f8f2));
        assert_eq!(palette.selection_bg, rgb(0x44475a));
        assert_eq!(palette.ansi[1], rgb(0xff5555));
    }

    #[test]
    fn normalize_scroll_delta_inverts_gtk_scroll_direction() {
        assert_eq!(normalize_scroll_delta(-1.0), 3);
        assert_eq!(normalize_scroll_delta(1.0), -3);
    }

    #[test]
    fn simple_selection_includes_both_endpoints() {
        let selection = simple_selection(
            Point::new(Line(0), Column(1)),
            Point::new(Line(0), Column(3)),
        );
        assert!(!selection.is_empty());
    }

    #[test]
    fn send_user_input_writes_bytes_and_notifies_callback() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(SharedWriter {
                written: written.clone(),
            })));
        let observed = Rc::new(RefCell::new(Vec::new()));
        let observed_cb = observed.clone();
        let input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>> =
            Rc::new(RefCell::new(Some(Rc::new(move |data| {
                observed_cb.borrow_mut().extend_from_slice(data);
            }))));

        assert!(send_user_input(&writer, &input_cb, b"echo hi\n"));
        assert_eq!(&*written.lock().expect("written bytes"), b"echo hi\n");
        assert_eq!(&*observed.borrow(), b"echo hi\n");
    }
}
