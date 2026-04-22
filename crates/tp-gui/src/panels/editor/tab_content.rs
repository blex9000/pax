//! Per-tab content in the Code Editor.
//!
//! Each open tab owns one `TabContent`. Source tabs hold a SourceView buffer
//! (the editor's shared source_view swaps to it on activation, matching the
//! pre-refactor behavior). Markdown tabs own their own widget tree
//! (rendered view + source view inside an inner stack) that lives as a child
//! of the editor's content_stack. Image tabs (Task 5) are analogous.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Data that's specific to a source-code tab.
#[derive(Debug, Clone)]
pub struct SourceTab {
    pub buffer: sourceview5::Buffer,
    pub modified: bool,
    /// Content on disk at last open/save — drives dirty detection.
    pub saved_content: Rc<RefCell<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkdownMode {
    Rendered,
    Source,
}

/// Data that's specific to a Markdown tab.
#[derive(Debug, Clone)]
pub struct MarkdownTab {
    pub buffer: sourceview5::Buffer,
    pub source_view: sourceview5::View,
    pub rendered_view: gtk4::TextView,
    /// Stack switching between rendered and source children.
    pub inner_stack: gtk4::Stack,
    pub mode: Rc<Cell<MarkdownMode>>,
    pub modified: bool,
    pub saved_content: Rc<RefCell<String>>,
    /// Outer widget that lives in the editor's content_stack under `tab-{id}`.
    pub outer: gtk4::Widget,
}

/// Data that's specific to an Image tab.
#[derive(Debug, Clone)]
pub struct ImageTab {
    pub picture: gtk4::Picture,
    /// Natural width in pixels (image's intrinsic size). 0 when unknown.
    pub natural_width: i32,
    /// Natural height in pixels. 0 when unknown.
    pub natural_height: i32,
    pub zoom: Rc<Cell<f64>>,
    /// Reset-zoom button label — handle is kept so keyboard shortcuts (in
    /// the editor's top-level key handler) can update the displayed "100%"
    /// after zoom-in/out.
    pub reset_button: gtk4::Button,
    pub outer: gtk4::Widget,
}

#[derive(Debug)]
pub enum TabContent {
    Source(SourceTab),
    Markdown(MarkdownTab),
    Image(ImageTab),
}

impl TabContent {
    /// Borrow the source-code buffer, or `None` for non-source tabs.
    pub fn source_buffer(&self) -> Option<&sourceview5::Buffer> {
        match self {
            TabContent::Source(s) => Some(&s.buffer),
            _ => None,
        }
    }

    /// Borrow a writable buffer (source or markdown source), or `None` for
    /// read-only tabs (image). Callers that need to save / track dirty state
    /// use this.
    pub fn writable_buffer(&self) -> Option<&sourceview5::Buffer> {
        match self {
            TabContent::Source(s) => Some(&s.buffer),
            TabContent::Markdown(m) => Some(&m.buffer),
            TabContent::Image(_) => None,
        }
    }

    pub fn is_modified(&self) -> bool {
        match self {
            TabContent::Source(s) => s.modified,
            TabContent::Markdown(m) => m.modified,
            TabContent::Image(_) => false,
        }
    }

    pub fn set_modified(&mut self, v: bool) {
        match self {
            TabContent::Source(s) => s.modified = v,
            TabContent::Markdown(m) => m.modified = v,
            TabContent::Image(_) => {}
        }
    }

    pub fn saved_content(&self) -> Option<&Rc<RefCell<String>>> {
        match self {
            TabContent::Source(s) => Some(&s.saved_content),
            TabContent::Markdown(m) => Some(&m.saved_content),
            TabContent::Image(_) => None,
        }
    }

    /// Name used as the child key under `content_stack` for this tab. Source
    /// tabs reuse the shared `"editor"` child; non-source tabs have their own
    /// per-tab widget keyed by `tab-{id}`.
    pub fn content_stack_child_name(&self, tab_id: u64) -> String {
        match self {
            TabContent::Source(_) => "editor".to_string(),
            _ => format!("tab-{}", tab_id),
        }
    }
}
