use gtk4::prelude::*;
use regex::Regex;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;
use std::sync::OnceLock;

const NODE_W: f64 = 132.0;
const NODE_H: f64 = 58.0;
const GRID_STEP: f64 = 32.0;
const ZOOM_MIN: f64 = 0.25;
const ZOOM_MAX: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MermaidShape {
    Rect,
    Round,
    Stadium,
    Diamond,
    Circle,
    Hexagon,
    Database,
    Parallelogram,
}

impl MermaidShape {
    fn label(self) -> &'static str {
        match self {
            Self::Rect => "Process",
            Self::Round => "Rounded",
            Self::Stadium => "Terminator",
            Self::Diamond => "Decision",
            Self::Circle => "Circle",
            Self::Hexagon => "Hexagon",
            Self::Database => "Database",
            Self::Parallelogram => "Input/Output",
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::Rect,
            Self::Round,
            Self::Stadium,
            Self::Diamond,
            Self::Circle,
            Self::Hexagon,
            Self::Database,
            Self::Parallelogram,
        ]
    }
}

#[derive(Debug, Clone)]
struct DesignerNode {
    id: String,
    label: String,
    shape: MermaidShape,
    x: f64,
    y: f64,
}

#[derive(Debug, Clone)]
struct DesignerEdge {
    from: usize,
    to: usize,
    label: String,
}

#[derive(Debug)]
struct DesignerState {
    nodes: Vec<DesignerNode>,
    edges: Vec<DesignerEdge>,
    selected_node: Option<usize>,
    selected_edge: Option<usize>,
    connect_from: Option<usize>,
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
    next_id: usize,
}

impl Default for DesignerState {
    fn default() -> Self {
        let mut state = Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            selected_node: None,
            selected_edge: None,
            connect_from: None,
            zoom: 1.0,
            pan_x: 120.0,
            pan_y: 90.0,
            next_id: 1,
        };
        let a = state.add_node(MermaidShape::Rect, "Start", 80.0, 80.0);
        let b = state.add_node(MermaidShape::Diamond, "Condition?", 300.0, 80.0);
        let c = state.add_node(MermaidShape::Rect, "Action", 520.0, 80.0);
        state.edges.push(DesignerEdge {
            from: a,
            to: b,
            label: String::new(),
        });
        state.edges.push(DesignerEdge {
            from: b,
            to: c,
            label: "Yes".to_string(),
        });
        state.selected_node = Some(a);
        state
    }
}

impl DesignerState {
    fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            selected_node: None,
            selected_edge: None,
            connect_from: None,
            zoom: 1.0,
            pan_x: 120.0,
            pan_y: 90.0,
            next_id: 1,
        }
    }

    fn add_node(&mut self, shape: MermaidShape, label: &str, x: f64, y: f64) -> usize {
        let id = format!("N{}", self.next_id);
        self.next_id += 1;
        let idx = self.nodes.len();
        self.nodes.push(DesignerNode {
            id,
            label: label.to_string(),
            shape,
            x,
            y,
        });
        self.selected_node = Some(idx);
        self.selected_edge = None;
        idx
    }

    fn add_imported_node(&mut self, id: String, label: String, shape: MermaidShape) -> usize {
        let idx = self.nodes.len();
        if let Some(n) = id.strip_prefix('N').and_then(|s| s.parse::<usize>().ok()) {
            self.next_id = self.next_id.max(n + 1);
        }
        self.nodes.push(DesignerNode {
            id,
            label,
            shape,
            x: 0.0,
            y: 0.0,
        });
        idx
    }

    fn delete_selection(&mut self) {
        if let Some(idx) = self.selected_node.take() {
            self.edges.retain(|edge| edge.from != idx && edge.to != idx);
            for edge in &mut self.edges {
                if edge.from > idx {
                    edge.from -= 1;
                }
                if edge.to > idx {
                    edge.to -= 1;
                }
            }
            self.nodes.remove(idx);
            self.selected_edge = None;
            self.connect_from = None;
            return;
        }
        if let Some(idx) = self.selected_edge.take() {
            if idx < self.edges.len() {
                self.edges.remove(idx);
            }
        }
    }

    fn select_node(&mut self, idx: usize) {
        self.selected_node = Some(idx);
        self.selected_edge = None;
    }

    fn select_edge(&mut self, idx: usize) {
        self.selected_node = None;
        self.selected_edge = Some(idx);
    }
}

#[derive(Debug, Clone, Copy)]
enum DragMode {
    None,
    Node {
        idx: usize,
        start_x: f64,
        start_y: f64,
    },
    Pan {
        start_x: f64,
        start_y: f64,
    },
}

#[derive(Debug, Clone, Copy)]
struct Rgba(f64, f64, f64, f64);

#[derive(Debug, Clone, Copy)]
struct Palette {
    canvas: Rgba,
    grid: Rgba,
    node_fill: Rgba,
    node_border: Rgba,
    selected: Rgba,
    edge: Rgba,
    text: Rgba,
    muted_text: Rgba,
    label_bg: Rgba,
}

pub fn show_mermaid_designer(parent: &impl IsA<gtk4::Window>) {
    show_mermaid_designer_internal(parent, None);
}

pub fn show_mermaid_designer_with_code(parent: &impl IsA<gtk4::Window>, source: &str) {
    show_mermaid_designer_internal(parent, Some(source));
}

fn show_mermaid_designer_internal(parent: &impl IsA<gtk4::Window>, source: Option<&str>) {
    let window = gtk4::Window::builder()
        .title("Mermaid Designer")
        .transient_for(parent)
        .default_width(1220)
        .default_height(760)
        .build();
    crate::theme::configure_dialog_window(&window);

    let initial_state = source
        .and_then(|source| parse_designer_state(source).ok())
        .unwrap_or_default();
    let initial_code = source.map(str::to_string);
    let state = Rc::new(RefCell::new(initial_state));
    let drag_mode = Rc::new(RefCell::new(DragMode::None));
    let connect_mode = Rc::new(Cell::new(false));
    let sync_guard = Rc::new(Cell::new(false));
    let code_dirty = Rc::new(Cell::new(false));
    let preserve_code_once = Rc::new(Cell::new(initial_code.is_some()));

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.set_vexpand(true);
    root.set_hexpand(true);

    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    toolbar.set_margin_top(8);
    toolbar.set_margin_bottom(8);
    toolbar.set_margin_start(10);
    toolbar.set_margin_end(10);
    toolbar.add_css_class("markdown-toolbar");
    root.append(&toolbar);

    let body = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    body.set_wide_handle(true);
    body.set_position(820);
    body.set_vexpand(true);
    body.set_hexpand(true);
    root.append(&body);

    let canvas = gtk4::DrawingArea::new();
    canvas.set_vexpand(true);
    canvas.set_hexpand(true);
    canvas.set_focusable(true);
    canvas.set_draw_func({
        let state = state.clone();
        move |_, cr, width, height| {
            paint_designer(cr, width, height, &state.borrow());
        }
    });

    let canvas_frame = gtk4::Frame::new(None);
    canvas_frame.set_margin_start(8);
    canvas_frame.set_margin_end(8);
    canvas_frame.set_margin_bottom(8);
    canvas_frame.set_child(Some(&canvas));
    body.set_start_child(Some(&canvas_frame));

    let side = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    side.set_margin_top(8);
    side.set_margin_bottom(8);
    side.set_margin_start(4);
    side.set_margin_end(10);
    side.set_size_request(330, -1);
    body.set_end_child(Some(&side));

    let inspector_title = gtk4::Label::new(Some("Selection"));
    inspector_title.set_halign(gtk4::Align::Start);
    inspector_title.add_css_class("heading");
    side.append(&inspector_title);

    let label_entry = gtk4::Entry::new();
    label_entry.set_placeholder_text(Some("Selected node/edge label"));
    side.append(&label_entry);

    let shape_buttons: Rc<RefCell<Vec<(MermaidShape, gtk4::Button)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let shape_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    for row_shapes in MermaidShape::all().chunks(2) {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        for shape in row_shapes {
            let btn = gtk4::Button::with_label(shape.label());
            btn.add_css_class("flat");
            btn.set_hexpand(true);
            row.append(&btn);
            shape_buttons.borrow_mut().push((*shape, btn));
        }
        shape_box.append(&row);
    }
    side.append(&shape_box);

    let delete_btn = gtk4::Button::with_label("Delete Selection");
    delete_btn.add_css_class("destructive-action");
    side.append(&delete_btn);

    let hint = gtk4::Label::new(Some(
        "Drag nodes to move them. Drag empty canvas to pan. Mouse wheel zooms. Enable Connect, then click source and target nodes.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    side.append(&hint);

    let code_title = gtk4::Label::new(Some("Mermaid Code"));
    code_title.set_halign(gtk4::Align::Start);
    code_title.add_css_class("heading");
    code_title.set_margin_top(10);
    side.append(&code_title);

    let code_buffer = gtk4::TextBuffer::new(None::<&gtk4::TextTagTable>);
    if let Some(initial_code) = initial_code.as_deref() {
        code_buffer.set_text(initial_code);
    }
    let code_view = gtk4::TextView::with_buffer(&code_buffer);
    code_view.set_editable(true);
    code_view.set_monospace(true);
    code_view.set_wrap_mode(gtk4::WrapMode::None);
    code_view.add_css_class("editor-code-view");

    let code_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let apply_code_btn = gtk4::Button::with_label("Apply Code");
    apply_code_btn.add_css_class("flat");
    apply_code_btn.set_tooltip_text(Some("Parse the Mermaid source into the visual canvas"));
    let reset_code_btn = gtk4::Button::with_label("Regenerate");
    reset_code_btn.add_css_class("flat");
    reset_code_btn.set_tooltip_text(Some(
        "Replace the source with code generated from the canvas",
    ));
    code_actions.append(&apply_code_btn);
    code_actions.append(&reset_code_btn);
    side.append(&code_actions);

    let code_status = gtk4::Label::new(None);
    code_status.set_halign(gtk4::Align::Start);
    code_status.add_css_class("dim-label");
    code_status.add_css_class("caption");
    side.append(&code_status);

    let code_scroll = gtk4::ScrolledWindow::new();
    code_scroll.set_vexpand(true);
    code_scroll.set_hexpand(true);
    code_scroll.set_min_content_height(220);
    code_scroll.set_child(Some(&code_view));
    side.append(&code_scroll);

    let refresh_ui: Rc<dyn Fn()> = {
        let state = state.clone();
        let canvas = canvas.clone();
        let code_buffer = code_buffer.clone();
        let inspector_title = inspector_title.clone();
        let label_entry = label_entry.clone();
        let shape_buttons = shape_buttons.clone();
        let delete_btn = delete_btn.clone();
        let connect_mode = connect_mode.clone();
        let sync_guard = sync_guard.clone();
        let code_dirty = code_dirty.clone();
        let preserve_code_once = preserve_code_once.clone();
        Rc::new(move || {
            let state_ref = state.borrow();
            sync_guard.set(true);
            if preserve_code_once.get() {
                preserve_code_once.set(false);
            } else if !code_dirty.get() {
                code_buffer.set_text(&generate_mermaid_code(&state_ref));
            }
            if let Some(idx) = state_ref.selected_node {
                if let Some(node) = state_ref.nodes.get(idx) {
                    inspector_title.set_text(&format!("Node {}", node.id));
                    label_entry.set_sensitive(true);
                    label_entry.set_placeholder_text(Some("Node label"));
                    label_entry.set_text(&node.label);
                    for (shape, btn) in shape_buttons.borrow().iter() {
                        btn.set_sensitive(true);
                        if *shape == node.shape {
                            btn.add_css_class("suggested-action");
                        } else {
                            btn.remove_css_class("suggested-action");
                        }
                    }
                    delete_btn.set_sensitive(true);
                }
            } else if let Some(idx) = state_ref.selected_edge {
                if let Some(edge) = state_ref.edges.get(idx) {
                    let from = state_ref
                        .nodes
                        .get(edge.from)
                        .map(|n| n.id.as_str())
                        .unwrap_or("?");
                    let to = state_ref
                        .nodes
                        .get(edge.to)
                        .map(|n| n.id.as_str())
                        .unwrap_or("?");
                    inspector_title.set_text(&format!("Edge {from} -> {to}"));
                    label_entry.set_sensitive(true);
                    label_entry.set_placeholder_text(Some("Edge label"));
                    label_entry.set_text(&edge.label);
                    for (_, btn) in shape_buttons.borrow().iter() {
                        btn.set_sensitive(false);
                        btn.remove_css_class("suggested-action");
                    }
                    delete_btn.set_sensitive(true);
                }
            } else {
                let title = if let Some(idx) = state_ref.connect_from {
                    state_ref
                        .nodes
                        .get(idx)
                        .map(|node| format!("Connect from {}", node.id))
                        .unwrap_or_else(|| "Selection".to_string())
                } else if connect_mode.get() {
                    "Connect: choose source".to_string()
                } else {
                    "Selection".to_string()
                };
                inspector_title.set_text(&title);
                label_entry.set_sensitive(false);
                label_entry.set_text("");
                for (_, btn) in shape_buttons.borrow().iter() {
                    btn.set_sensitive(false);
                    btn.remove_css_class("suggested-action");
                }
                delete_btn.set_sensitive(false);
            }
            sync_guard.set(false);
            canvas.queue_draw();
        })
    };

    {
        let sync_guard = sync_guard.clone();
        let code_dirty = code_dirty.clone();
        code_buffer.connect_changed(move |_| {
            if !sync_guard.get() {
                code_dirty.set(true);
            }
        });
    }

    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        let code_buffer = code_buffer.clone();
        let code_dirty = code_dirty.clone();
        let code_status = code_status.clone();
        apply_code_btn.connect_clicked(move |_| {
            let source = code_buffer
                .text(&code_buffer.start_iter(), &code_buffer.end_iter(), false)
                .to_string();
            match parse_designer_state(&source) {
                Ok(parsed) => {
                    *state.borrow_mut() = parsed;
                    code_dirty.set(false);
                    code_status.set_text("Imported Mermaid flowchart into the canvas.");
                    refresh();
                }
                Err(e) => {
                    code_status.set_text(&format!("Cannot import visually: {e}"));
                }
            }
        });
    }

    {
        let refresh = refresh_ui.clone();
        let code_dirty = code_dirty.clone();
        let code_status = code_status.clone();
        reset_code_btn.connect_clicked(move |_| {
            code_dirty.set(false);
            code_status.set_text("Code regenerated from the canvas.");
            refresh();
        });
    }

    // Shape add buttons.
    for shape in MermaidShape::all() {
        let btn = gtk4::Button::with_label(shape.label());
        btn.add_css_class("flat");
        toolbar.append(&btn);
        let state_c = state.clone();
        let canvas_c = canvas.clone();
        let refresh = refresh_ui.clone();
        let shape = *shape;
        btn.connect_clicked(move |_| {
            let width = canvas_c.allocated_width().max(1) as f64;
            let height = canvas_c.allocated_height().max(1) as f64;
            {
                let mut st = state_c.borrow_mut();
                let x = (width / 2.0 - st.pan_x) / st.zoom;
                let y = (height / 2.0 - st.pan_y) / st.zoom;
                st.add_node(shape, shape.label(), x, y);
            }
            refresh();
        });
    }

    let separator = gtk4::Separator::new(gtk4::Orientation::Vertical);
    toolbar.append(&separator);

    let connect_btn = gtk4::ToggleButton::with_label("Connect");
    connect_btn.add_css_class("flat");
    connect_btn.set_tooltip_text(Some("Click source node, then target node"));
    toolbar.append(&connect_btn);
    {
        let state = state.clone();
        let connect_mode = connect_mode.clone();
        let refresh = refresh_ui.clone();
        connect_btn.connect_toggled(move |btn| {
            connect_mode.set(btn.is_active());
            state.borrow_mut().connect_from = None;
            refresh();
        });
    }

    let zoom_out_btn = gtk4::Button::from_icon_name("zoom-out-symbolic");
    zoom_out_btn.add_css_class("flat");
    toolbar.append(&zoom_out_btn);
    let zoom_reset_btn = gtk4::Button::with_label("100%");
    zoom_reset_btn.add_css_class("flat");
    toolbar.append(&zoom_reset_btn);
    let zoom_in_btn = gtk4::Button::from_icon_name("zoom-in-symbolic");
    zoom_in_btn.add_css_class("flat");
    toolbar.append(&zoom_in_btn);

    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        zoom_out_btn.connect_clicked(move |_| {
            let mut st = state.borrow_mut();
            st.zoom = (st.zoom / 1.2).clamp(ZOOM_MIN, ZOOM_MAX);
            refresh();
        });
    }
    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        zoom_in_btn.connect_clicked(move |_| {
            let mut st = state.borrow_mut();
            st.zoom = (st.zoom * 1.2).clamp(ZOOM_MIN, ZOOM_MAX);
            refresh();
        });
    }
    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        zoom_reset_btn.connect_clicked(move |_| {
            let mut st = state.borrow_mut();
            st.zoom = 1.0;
            st.pan_x = 120.0;
            st.pan_y = 90.0;
            refresh();
        });
    }

    let copy_btn = gtk4::Button::with_label("Copy Code");
    copy_btn.add_css_class("suggested-action");
    toolbar.append(&copy_btn);
    {
        let state = state.clone();
        let window = window.clone();
        copy_btn.connect_clicked(move |_| {
            window
                .clipboard()
                .set_text(&generate_mermaid_code(&state.borrow()));
        });
    }

    let clear_btn = gtk4::Button::with_label("Clear");
    clear_btn.add_css_class("destructive-action");
    toolbar.append(&clear_btn);
    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        clear_btn.connect_clicked(move |_| {
            let mut st = state.borrow_mut();
            st.nodes.clear();
            st.edges.clear();
            st.selected_node = None;
            st.selected_edge = None;
            st.connect_from = None;
            st.next_id = 1;
            refresh();
        });
    }

    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        let sync_guard = sync_guard.clone();
        label_entry.connect_changed(move |entry| {
            if sync_guard.get() {
                return;
            }
            let mut st = state.borrow_mut();
            let text = entry.text().to_string();
            if let Some(idx) = st.selected_node {
                if let Some(node) = st.nodes.get_mut(idx) {
                    node.label = text;
                }
            } else if let Some(idx) = st.selected_edge {
                if let Some(edge) = st.edges.get_mut(idx) {
                    edge.label = text;
                }
            }
            drop(st);
            refresh();
        });
    }

    for (shape, btn) in shape_buttons.borrow().iter().cloned() {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        let sync_guard = sync_guard.clone();
        btn.connect_clicked(move |_| {
            if sync_guard.get() {
                return;
            }
            let mut st = state.borrow_mut();
            if let Some(idx) = st.selected_node {
                if let Some(node) = st.nodes.get_mut(idx) {
                    node.shape = shape;
                }
            }
            drop(st);
            refresh();
        });
    }

    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        delete_btn.connect_clicked(move |_| {
            state.borrow_mut().delete_selection();
            refresh();
        });
    }

    install_canvas_input(
        &canvas,
        state.clone(),
        drag_mode,
        connect_mode,
        refresh_ui.clone(),
    );

    window.set_child(Some(&root));
    refresh_ui();
    window.present();
}

fn install_canvas_input(
    canvas: &gtk4::DrawingArea,
    state: Rc<RefCell<DesignerState>>,
    drag_mode: Rc<RefCell<DragMode>>,
    connect_mode: Rc<Cell<bool>>,
    refresh_ui: Rc<dyn Fn()>,
) {
    let click = gtk4::GestureClick::new();
    click.set_button(0);
    {
        let state = state.clone();
        let connect_mode = connect_mode.clone();
        let refresh = refresh_ui.clone();
        click.connect_pressed(move |_, _, x, y| {
            let (mx, my) = {
                let st = state.borrow();
                screen_to_model(&st, x, y)
            };
            let hit = {
                let st = state.borrow();
                hit_node(&st, mx, my)
            };

            if connect_mode.get() {
                if let Some(idx) = hit {
                    let mut st = state.borrow_mut();
                    if let Some(from) = st.connect_from.take() {
                        if from != idx
                            && !st
                                .edges
                                .iter()
                                .any(|edge| edge.from == from && edge.to == idx)
                        {
                            st.edges.push(DesignerEdge {
                                from,
                                to: idx,
                                label: String::new(),
                            });
                        }
                        st.select_node(idx);
                    } else {
                        st.connect_from = Some(idx);
                        st.select_node(idx);
                    }
                }
                refresh();
                return;
            }

            let mut st = state.borrow_mut();
            if let Some(idx) = hit {
                st.select_node(idx);
            } else if let Some(edge_idx) = hit_edge(&st, mx, my) {
                st.select_edge(edge_idx);
            } else {
                st.selected_node = None;
                st.selected_edge = None;
            }
            drop(st);
            refresh();
        });
    }
    canvas.add_controller(click);

    let drag = gtk4::GestureDrag::new();
    {
        let state = state.clone();
        let drag_mode = drag_mode.clone();
        let connect_mode = connect_mode.clone();
        drag.connect_drag_begin(move |_, x, y| {
            if connect_mode.get() {
                *drag_mode.borrow_mut() = DragMode::None;
                return;
            }
            let st = state.borrow();
            let (mx, my) = screen_to_model(&st, x, y);
            *drag_mode.borrow_mut() = if let Some(idx) = hit_node(&st, mx, my) {
                let node = &st.nodes[idx];
                DragMode::Node {
                    idx,
                    start_x: node.x,
                    start_y: node.y,
                }
            } else {
                DragMode::Pan {
                    start_x: st.pan_x,
                    start_y: st.pan_y,
                }
            };
        });
    }
    {
        let state = state.clone();
        let drag_mode = drag_mode.clone();
        let refresh = refresh_ui.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let mode = *drag_mode.borrow();
            let mut st = state.borrow_mut();
            match mode {
                DragMode::Node {
                    idx,
                    start_x,
                    start_y,
                } => {
                    let zoom = st.zoom;
                    if let Some(node) = st.nodes.get_mut(idx) {
                        node.x = start_x + dx / zoom;
                        node.y = start_y + dy / zoom;
                    }
                }
                DragMode::Pan { start_x, start_y } => {
                    st.pan_x = start_x + dx;
                    st.pan_y = start_y + dy;
                }
                DragMode::None => {}
            }
            drop(st);
            refresh();
        });
    }
    {
        let drag_mode = drag_mode.clone();
        drag.connect_drag_end(move |_, _, _| {
            *drag_mode.borrow_mut() = DragMode::None;
        });
    }
    canvas.add_controller(drag);

    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::VERTICAL | gtk4::EventControllerScrollFlags::DISCRETE,
    );
    {
        let state = state.clone();
        let refresh = refresh_ui.clone();
        scroll.connect_scroll(move |_, _dx, dy| {
            let mut st = state.borrow_mut();
            let factor = if dy < 0.0 { 1.12 } else { 1.0 / 1.12 };
            st.zoom = (st.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
            drop(st);
            refresh();
            gtk4::glib::Propagation::Stop
        });
    }
    canvas.add_controller(scroll);
}

fn paint_designer(cr: &gtk4::cairo::Context, width: i32, height: i32, state: &DesignerState) {
    let palette = current_palette();
    set_source(cr, palette.canvas);
    cr.rectangle(0.0, 0.0, width as f64, height as f64);
    let _ = cr.fill();

    let _ = cr.save();
    cr.translate(state.pan_x, state.pan_y);
    cr.scale(state.zoom, state.zoom);

    paint_grid(cr, width, height, state, palette);

    for (idx, edge) in state.edges.iter().enumerate() {
        paint_edge(cr, state, edge, state.selected_edge == Some(idx), palette);
    }
    for (idx, node) in state.nodes.iter().enumerate() {
        let selected = state.selected_node == Some(idx) || state.connect_from == Some(idx);
        paint_node(cr, node, selected, palette);
    }

    let _ = cr.restore();
}

fn paint_grid(
    cr: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    state: &DesignerState,
    p: Palette,
) {
    let left = -state.pan_x / state.zoom;
    let top = -state.pan_y / state.zoom;
    let right = left + width as f64 / state.zoom;
    let bottom = top + height as f64 / state.zoom;
    let start_x = (left / GRID_STEP).floor() as i32 - 1;
    let end_x = (right / GRID_STEP).ceil() as i32 + 1;
    let start_y = (top / GRID_STEP).floor() as i32 - 1;
    let end_y = (bottom / GRID_STEP).ceil() as i32 + 1;

    set_source(cr, p.grid);
    cr.set_line_width(1.0 / state.zoom);
    for x in start_x..=end_x {
        let px = x as f64 * GRID_STEP;
        cr.move_to(px, top - GRID_STEP);
        cr.line_to(px, bottom + GRID_STEP);
    }
    for y in start_y..=end_y {
        let py = y as f64 * GRID_STEP;
        cr.move_to(left - GRID_STEP, py);
        cr.line_to(right + GRID_STEP, py);
    }
    let _ = cr.stroke();
}

fn paint_node(cr: &gtk4::cairo::Context, node: &DesignerNode, selected: bool, p: Palette) {
    let (w, h) = node_size(node);
    let x = node.x - w / 2.0;
    let y = node.y - h / 2.0;
    draw_shape_path(cr, node.shape, x, y, w, h);
    set_source(cr, p.node_fill);
    let _ = cr.fill_preserve();
    set_source(cr, if selected { p.selected } else { p.node_border });
    cr.set_line_width(if selected { 2.8 } else { 1.4 });
    let _ = cr.stroke();

    paint_centered_text(cr, &node.label, node.x, node.y, w - 22.0, p.text);
}

fn paint_edge(
    cr: &gtk4::cairo::Context,
    state: &DesignerState,
    edge: &DesignerEdge,
    selected: bool,
    p: Palette,
) {
    let Some(from) = state.nodes.get(edge.from) else {
        return;
    };
    let Some(to) = state.nodes.get(edge.to) else {
        return;
    };
    let (sx, sy, ex, ey) = edge_endpoints(from, to);
    set_source(cr, if selected { p.selected } else { p.edge });
    cr.set_line_width(if selected { 2.5 } else { 1.5 });
    cr.move_to(sx, sy);
    let mid_x = (sx + ex) / 2.0;
    let mid_y = (sy + ey) / 2.0;
    cr.curve_to(mid_x, sy, mid_x, ey, ex, ey);
    let _ = cr.stroke();
    paint_arrowhead(
        cr,
        (ex, ey),
        (mid_x, mid_y),
        if selected { p.selected } else { p.edge },
    );

    if !edge.label.trim().is_empty() {
        paint_edge_label(cr, mid_x, mid_y, &edge.label, p);
    }
}

fn draw_shape_path(cr: &gtk4::cairo::Context, shape: MermaidShape, x: f64, y: f64, w: f64, h: f64) {
    match shape {
        MermaidShape::Rect => rounded_rect(cr, x, y, w, h, 6.0),
        MermaidShape::Round => rounded_rect(cr, x, y, w, h, 14.0),
        MermaidShape::Stadium => rounded_rect(cr, x, y, w, h, h / 2.0),
        MermaidShape::Diamond => {
            cr.move_to(x + w / 2.0, y);
            cr.line_to(x + w, y + h / 2.0);
            cr.line_to(x + w / 2.0, y + h);
            cr.line_to(x, y + h / 2.0);
            cr.close_path();
        }
        MermaidShape::Circle => {
            let _ = cr.save();
            cr.translate(x + w / 2.0, y + h / 2.0);
            cr.scale(w / 2.0, h / 2.0);
            cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
            let _ = cr.restore();
        }
        MermaidShape::Hexagon => {
            cr.move_to(x + w * 0.24, y);
            cr.line_to(x + w * 0.76, y);
            cr.line_to(x + w, y + h / 2.0);
            cr.line_to(x + w * 0.76, y + h);
            cr.line_to(x + w * 0.24, y + h);
            cr.line_to(x, y + h / 2.0);
            cr.close_path();
        }
        MermaidShape::Database => {
            rounded_rect(cr, x, y, w, h, 10.0);
        }
        MermaidShape::Parallelogram => {
            let skew = 18.0;
            cr.move_to(x + skew, y);
            cr.line_to(x + w, y);
            cr.line_to(x + w - skew, y + h);
            cr.line_to(x, y + h);
            cr.close_path();
        }
    }
}

fn node_size(node: &DesignerNode) -> (f64, f64) {
    let max_line = node.label.lines().map(|line| line.len()).max().unwrap_or(1) as f64;
    let lines = node.label.lines().count().max(1) as f64;
    let mut w = (max_line * 8.0 + 42.0).clamp(NODE_W, 260.0);
    let mut h = (lines * 18.0 + 28.0).clamp(NODE_H, 130.0);
    match node.shape {
        MermaidShape::Diamond | MermaidShape::Hexagon => {
            w += 28.0;
            h = h.max(74.0);
        }
        MermaidShape::Circle => {
            let side = w.max(h).max(82.0);
            w = side;
            h = side;
        }
        MermaidShape::Rect
        | MermaidShape::Round
        | MermaidShape::Stadium
        | MermaidShape::Database
        | MermaidShape::Parallelogram => {}
    }
    (w, h)
}

fn edge_endpoints(from: &DesignerNode, to: &DesignerNode) -> (f64, f64, f64, f64) {
    let (fw, fh) = node_size(from);
    let (tw, th) = node_size(to);
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx.abs() >= dy.abs() {
        let sx = from.x + fw / 2.0 * dx.signum();
        let ex = to.x - tw / 2.0 * dx.signum();
        (sx, from.y, ex, to.y)
    } else {
        let sy = from.y + fh / 2.0 * dy.signum();
        let ey = to.y - th / 2.0 * dy.signum();
        (from.x, sy, to.x, ey)
    }
}

fn paint_centered_text(
    cr: &gtk4::cairo::Context,
    text: &str,
    x: f64,
    y: f64,
    width: f64,
    color: Rgba,
) {
    let layout = pangocairo::functions::create_layout(cr);
    let mut font = gtk4::pango::FontDescription::from_string("Sans 10");
    font.set_weight(gtk4::pango::Weight::Medium);
    layout.set_font_description(Some(&font));
    layout.set_alignment(gtk4::pango::Alignment::Center);
    layout.set_wrap(gtk4::pango::WrapMode::WordChar);
    layout.set_width((width.max(20.0) * gtk4::pango::SCALE as f64) as i32);
    layout.set_text(text);
    let (tw, th) = layout.pixel_size();
    set_source(cr, color);
    cr.move_to(x - tw as f64 / 2.0, y - th as f64 / 2.0);
    pangocairo::functions::show_layout(cr, &layout);
}

fn paint_edge_label(cr: &gtk4::cairo::Context, x: f64, y: f64, text: &str, p: Palette) {
    let layout = pangocairo::functions::create_layout(cr);
    let font = gtk4::pango::FontDescription::from_string("Sans 9");
    layout.set_font_description(Some(&font));
    layout.set_text(text);
    let (tw, th) = layout.pixel_size();
    rounded_rect(
        cr,
        x - tw as f64 / 2.0 - 6.0,
        y - th as f64 / 2.0 - 3.0,
        tw as f64 + 12.0,
        th as f64 + 6.0,
        7.0,
    );
    set_source(cr, p.label_bg);
    let _ = cr.fill();
    set_source(cr, p.muted_text);
    cr.move_to(x - tw as f64 / 2.0, y - th as f64 / 2.0);
    pangocairo::functions::show_layout(cr, &layout);
}

fn paint_arrowhead(cr: &gtk4::cairo::Context, end: (f64, f64), prev: (f64, f64), color: Rgba) {
    let angle = (end.1 - prev.1).atan2(end.0 - prev.0);
    let size = 9.0;
    set_source(cr, color);
    cr.move_to(end.0, end.1);
    cr.line_to(
        end.0 + size * (angle + std::f64::consts::PI * 0.82).cos(),
        end.1 + size * (angle + std::f64::consts::PI * 0.82).sin(),
    );
    cr.line_to(
        end.0 + size * (angle - std::f64::consts::PI * 0.82).cos(),
        end.1 + size * (angle - std::f64::consts::PI * 0.82).sin(),
    );
    cr.close_path();
    let _ = cr.fill();
}

fn rounded_rect(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        std::f64::consts::PI * 1.5,
    );
    cr.close_path();
}

fn current_palette() -> Palette {
    match crate::theme::current_theme().to_id() {
        "aurora" | "quantum" => Palette {
            canvas: Rgba(0.965, 0.976, 0.992, 1.0),
            grid: Rgba(0.78, 0.84, 0.91, 0.55),
            node_fill: Rgba(1.0, 1.0, 1.0, 1.0),
            node_border: Rgba(0.16, 0.43, 0.66, 1.0),
            selected: Rgba(0.0, 0.65, 0.82, 1.0),
            edge: Rgba(0.33, 0.40, 0.52, 1.0),
            text: Rgba(0.06, 0.11, 0.20, 1.0),
            muted_text: Rgba(0.34, 0.40, 0.52, 1.0),
            label_bg: Rgba(0.965, 0.976, 0.992, 0.94),
        },
        _ => Palette {
            canvas: Rgba(0.055, 0.075, 0.105, 1.0),
            grid: Rgba(0.25, 0.32, 0.40, 0.45),
            node_fill: Rgba(0.105, 0.135, 0.18, 1.0),
            node_border: Rgba(0.22, 0.72, 0.86, 1.0),
            selected: Rgba(0.0, 0.78, 0.95, 1.0),
            edge: Rgba(0.62, 0.70, 0.80, 1.0),
            text: Rgba(0.90, 0.93, 0.96, 1.0),
            muted_text: Rgba(0.70, 0.77, 0.84, 1.0),
            label_bg: Rgba(0.055, 0.075, 0.105, 0.94),
        },
    }
}

fn set_source(cr: &gtk4::cairo::Context, color: Rgba) {
    cr.set_source_rgba(color.0, color.1, color.2, color.3);
}

fn screen_to_model(state: &DesignerState, x: f64, y: f64) -> (f64, f64) {
    (
        (x - state.pan_x) / state.zoom,
        (y - state.pan_y) / state.zoom,
    )
}

fn hit_node(state: &DesignerState, x: f64, y: f64) -> Option<usize> {
    state
        .nodes
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, node)| {
            let (w, h) = node_size(node);
            let hit = x >= node.x - w / 2.0
                && x <= node.x + w / 2.0
                && y >= node.y - h / 2.0
                && y <= node.y + h / 2.0;
            hit.then_some(idx)
        })
}

fn hit_edge(state: &DesignerState, x: f64, y: f64) -> Option<usize> {
    state.edges.iter().enumerate().find_map(|(idx, edge)| {
        let from = state.nodes.get(edge.from)?;
        let to = state.nodes.get(edge.to)?;
        let distance = distance_to_segment((x, y), (from.x, from.y), (to.x, to.y));
        (distance <= 10.0).then_some(idx)
    })
}

fn distance_to_segment(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let wx = point.0 - a.0;
    let wy = point.1 - a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= f64::EPSILON {
        return ((point.0 - a.0).powi(2) + (point.1 - a.1).powi(2)).sqrt();
    }
    let t = ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0);
    let proj = (a.0 + t * vx, a.1 + t * vy);
    ((point.0 - proj.0).powi(2) + (point.1 - proj.1).powi(2)).sqrt()
}

#[derive(Debug, Clone)]
struct ParsedDesignerNodeRef {
    id: String,
    label: Option<String>,
    shape: MermaidShape,
}

#[derive(Default)]
struct DesignerImportBuilder {
    state: DesignerState,
    node_index: HashMap<String, usize>,
}

impl DesignerImportBuilder {
    fn ensure_node(&mut self, node_ref: ParsedDesignerNodeRef) -> usize {
        if let Some(idx) = self.node_index.get(&node_ref.id).copied() {
            let has_explicit_label = node_ref.label.is_some();
            if let Some(label) = node_ref.label {
                if !label.is_empty() {
                    self.state.nodes[idx].label = label;
                }
            }
            if has_explicit_label || node_ref.shape != MermaidShape::Rect {
                self.state.nodes[idx].shape = node_ref.shape;
            }
            return idx;
        }

        let idx = self.state.add_imported_node(
            node_ref.id.clone(),
            node_ref.label.unwrap_or_else(|| node_ref.id.clone()),
            node_ref.shape,
        );
        self.node_index.insert(node_ref.id, idx);
        idx
    }
}

fn parse_designer_state(source: &str) -> Result<DesignerState, String> {
    let mut builder = DesignerImportBuilder {
        state: DesignerState::empty(),
        node_index: HashMap::new(),
    };
    let mut saw_flowchart = false;

    for raw_line in source.lines() {
        let line = raw_line.split("%%").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        for statement in line.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            if parse_flowchart_header(statement) {
                saw_flowchart = true;
                continue;
            }
            if !saw_flowchart && is_unsupported_diagram_header(statement) {
                return Err(
                    "visual import currently supports Mermaid flowchart/graph blocks".to_string(),
                );
            }
            if is_ignored_import_statement(statement) {
                continue;
            }
            if let Some((from, label, to)) = parse_import_edge(statement) {
                let from = builder.ensure_node(from);
                let to = builder.ensure_node(to);
                builder.state.edges.push(DesignerEdge { from, to, label });
                continue;
            }
            if let Some(node_ref) = parse_import_node_ref(statement) {
                builder.ensure_node(node_ref);
            }
        }
    }

    if builder.state.nodes.is_empty() {
        return Err("no flowchart nodes found".to_string());
    }

    layout_imported_state(&mut builder.state);
    builder.state.selected_node = Some(0);
    Ok(builder.state)
}

fn parse_flowchart_header(statement: &str) -> bool {
    statement.split_whitespace().next().is_some_and(|kind| {
        kind.eq_ignore_ascii_case("flowchart") || kind.eq_ignore_ascii_case("graph")
    })
}

fn is_unsupported_diagram_header(statement: &str) -> bool {
    let first = statement.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "sequenceDiagram"
            | "classDiagram"
            | "stateDiagram"
            | "stateDiagram-v2"
            | "erDiagram"
            | "journey"
            | "gantt"
            | "pie"
            | "gitGraph"
            | "mindmap"
            | "timeline"
            | "quadrantChart"
            | "requirementDiagram"
            | "C4Context"
    )
}

fn is_ignored_import_statement(statement: &str) -> bool {
    let first = statement.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "subgraph" | "end" | "classDef" | "class" | "style" | "linkStyle" | "click"
    )
}

fn parse_import_edge(
    statement: &str,
) -> Option<(ParsedDesignerNodeRef, String, ParsedDesignerNodeRef)> {
    for regex in [
        import_pipe_edge_re(),
        import_text_edge_re(),
        import_plain_edge_re(),
    ] {
        if let Some(caps) = regex.captures(statement) {
            let from = parse_import_node_ref(caps.name("from")?.as_str())?;
            let to = parse_import_node_ref(caps.name("to")?.as_str())?;
            let label = caps
                .name("label")
                .map(|m| clean_import_label(m.as_str()))
                .unwrap_or_default();
            return Some((from, label, to));
        }
    }
    None
}

fn import_pipe_edge_re() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(r#"^(?P<from>.+?)\s*(?:-->|==>|-\.->|---)\s*\|(?P<label>[^|]+)\|\s*(?P<to>.+)$"#)
            .expect("valid Mermaid pipe edge regex")
    })
}

fn import_text_edge_re() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(r#"^(?P<from>.+?)\s+--\s+(?P<label>.+?)\s+--?>\s*(?P<to>.+)$"#)
            .expect("valid Mermaid text edge regex")
    })
}

fn import_plain_edge_re() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(r#"^(?P<from>.+?)\s*(?:-->|==>|-\.->|---)\s*(?P<to>.+)$"#)
            .expect("valid Mermaid plain edge regex")
    })
}

fn parse_import_node_ref(raw: &str) -> Option<ParsedDesignerNodeRef> {
    let raw = raw
        .trim()
        .trim_end_matches(';')
        .split(":::")
        .next()
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return None;
    }

    for (open, close, shape) in [
        ("((", "))", MermaidShape::Circle),
        ("{{", "}}", MermaidShape::Hexagon),
        ("[(", ")]", MermaidShape::Database),
        ("([", "])", MermaidShape::Stadium),
        ("[/", "/]", MermaidShape::Parallelogram),
        ("{", "}", MermaidShape::Diamond),
        ("[", "]", MermaidShape::Rect),
        ("(", ")", MermaidShape::Round),
    ] {
        if let Some(open_idx) = raw.find(open) {
            if raw.ends_with(close) {
                let id = raw[..open_idx].trim();
                if id.is_empty() {
                    return None;
                }
                let label_start = open_idx + open.len();
                let label_end = raw.len().saturating_sub(close.len());
                return Some(ParsedDesignerNodeRef {
                    id: id.to_string(),
                    label: Some(clean_import_label(&raw[label_start..label_end])),
                    shape,
                });
            }
        }
    }

    let id = raw
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| c == '"' || c == '\'');
    if id.is_empty() {
        return None;
    }
    Some(ParsedDesignerNodeRef {
        id: id.to_string(),
        label: None,
        shape: MermaidShape::Rect,
    })
}

fn clean_import_label(label: &str) -> String {
    label
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
}

fn layout_imported_state(state: &mut DesignerState) {
    let layers = imported_layers(state);
    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (idx, layer) in layers.iter().copied().enumerate() {
        grouped.entry(layer).or_default().push(idx);
    }

    let mut y = 80.0;
    for nodes in grouped.values() {
        let mut x = 90.0;
        for idx in nodes {
            if let Some(node) = state.nodes.get_mut(*idx) {
                node.x = x;
                node.y = y;
            }
            x += 220.0;
        }
        y += 140.0;
    }
}

fn imported_layers(state: &DesignerState) -> Vec<usize> {
    let n = state.nodes.len();
    let mut outgoing = vec![Vec::<usize>::new(); n];
    let mut indegree = vec![0_usize; n];
    for edge in &state.edges {
        if edge.from < n && edge.to < n {
            outgoing[edge.from].push(edge.to);
            indegree[edge.to] += 1;
        }
    }
    let mut queue: VecDeque<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(idx, degree)| (*degree == 0).then_some(idx))
        .collect();
    if queue.is_empty() && n > 0 {
        queue.push_back(0);
    }

    let mut layers = vec![0_usize; n];
    let mut indegree_work = indegree;
    let mut visited = vec![false; n];
    while let Some(idx) = queue.pop_front() {
        if visited[idx] {
            continue;
        }
        visited[idx] = true;
        for to in outgoing[idx].iter().copied() {
            layers[to] = layers[to].max(layers[idx] + 1);
            indegree_work[to] = indegree_work[to].saturating_sub(1);
            if indegree_work[to] == 0 {
                queue.push_back(to);
            }
        }
    }
    layers
}

fn generate_mermaid_code(state: &DesignerState) -> String {
    let mut lines = vec!["flowchart TD".to_string()];
    if state.edges.is_empty() {
        for node in &state.nodes {
            lines.push(format!("    {}", node_syntax(node)));
        }
    } else {
        for edge in &state.edges {
            let Some(from) = state.nodes.get(edge.from) else {
                continue;
            };
            let Some(to) = state.nodes.get(edge.to) else {
                continue;
            };
            if edge.label.trim().is_empty() {
                lines.push(format!("    {} --> {}", node_syntax(from), node_syntax(to)));
            } else {
                lines.push(format!(
                    "    {} -->|{}| {}",
                    node_syntax(from),
                    escape_mermaid_label(&edge.label),
                    node_syntax(to)
                ));
            }
        }
        for (idx, node) in state.nodes.iter().enumerate() {
            let referenced = state
                .edges
                .iter()
                .any(|edge| edge.from == idx || edge.to == idx);
            if !referenced {
                lines.push(format!("    {}", node_syntax(node)));
            }
        }
    }
    lines.join("\n")
}

fn node_syntax(node: &DesignerNode) -> String {
    let id = &node.id;
    let label = escape_mermaid_label(&node.label);
    match node.shape {
        MermaidShape::Rect => format!("{id}[{label}]"),
        MermaidShape::Round => format!("{id}({label})"),
        MermaidShape::Stadium => format!("{id}([{label}])"),
        MermaidShape::Diamond => format!("{id}{{{label}}}"),
        MermaidShape::Circle => format!("{id}(({label}))"),
        MermaidShape::Hexagon => format!("{id}{{{{{label}}}}}"),
        MermaidShape::Database => format!("{id}[({label})]"),
        MermaidShape::Parallelogram => format!("{id}[/{label}/]"),
    }
}

fn escape_mermaid_label(label: &str) -> String {
    label
        .replace('|', "/")
        .replace('[', "(")
        .replace(']', ")")
        .replace('{', "(")
        .replace('}', ")")
        .replace('\n', "<br/>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_includes_nodes_edges_and_labels() {
        let mut state = DesignerState::default();
        state.edges[1].label = "Sì".to_string();

        let code = generate_mermaid_code(&state);

        assert!(code.starts_with("flowchart TD"));
        assert!(code.contains("N1[Start] --> N2{Condition?}"));
        assert!(code.contains("N2{Condition?} -->|Sì| N3[Action]"));
    }

    #[test]
    fn delete_node_reindexes_edges() {
        let mut state = DesignerState::default();
        state.selected_node = Some(1);
        state.delete_selection();

        assert_eq!(state.nodes.len(), 2);
        assert!(state.edges.iter().all(|edge| edge.from < 2 && edge.to < 2));
    }

    #[test]
    fn imports_existing_flowchart_code() {
        let state = parse_designer_state(
            r#"
flowchart TD
    A[Inizio] --> B{Condizione?}
    B -- Sì --> C[Azione 1]
    B -- No --> D[Azione 2]
"#,
        )
        .unwrap();

        assert_eq!(state.nodes.len(), 4);
        assert_eq!(state.edges.len(), 3);
        assert_eq!(state.nodes[1].shape, MermaidShape::Diamond);
        assert!(state.edges.iter().any(|edge| edge.label == "Sì"));
    }

    #[test]
    fn rejects_visual_import_for_non_flowchart_mermaid() {
        let err = parse_designer_state(
            r#"
sequenceDiagram
    Alice->>Bob: Hello
"#,
        )
        .unwrap_err();

        assert!(err.contains("flowchart/graph"));
    }
}
