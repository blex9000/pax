//! Per-tab content in the Code Editor.
//!
//! Each open tab owns one `TabContent`. Source tabs hold a SourceView buffer,
//! Markdown tabs a rendered/source toggle (wired in Task 4), Image tabs a
//! picture + zoom state (wired in Task 5).

use std::cell::RefCell;
use std::rc::Rc;

/// Data that's specific to a source-code tab.
#[derive(Debug, Clone)]
pub struct SourceTab {
    pub buffer: sourceview5::Buffer,
    pub modified: bool,
    /// Content on disk at last open/save — drives dirty detection.
    pub saved_content: Rc<RefCell<String>>,
}

/// Data that's specific to a Markdown tab. Populated in Task 4.
#[derive(Debug, Default)]
pub struct MarkdownTab {}

/// Data that's specific to an Image tab. Populated in Task 5.
#[derive(Debug, Default)]
pub struct ImageTab {}

#[derive(Debug)]
pub enum TabContent {
    Source(SourceTab),
    Markdown(MarkdownTab),
    Image(ImageTab),
}

impl TabContent {
    /// Borrow the source buffer, or `None` for non-source tabs.
    pub fn source_buffer(&self) -> Option<&sourceview5::Buffer> {
        match self {
            TabContent::Source(s) => Some(&s.buffer),
            _ => None,
        }
    }

    pub fn is_modified(&self) -> bool {
        match self {
            TabContent::Source(s) => s.modified,
            _ => false,
        }
    }

    pub fn set_modified(&mut self, v: bool) {
        if let TabContent::Source(s) = self {
            s.modified = v;
        }
    }

    /// Borrow the dirty-tracking saved-content cell, or `None` for non-writable
    /// tabs (image tabs). Markdown tabs get their own cell in Task 4.
    pub fn saved_content(&self) -> Option<&Rc<RefCell<String>>> {
        match self {
            TabContent::Source(s) => Some(&s.saved_content),
            _ => None,
        }
    }
}
