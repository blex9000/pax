use std::collections::HashMap;

use crate::panel_host::PanelHost;

/// Manages focus state across panels.
pub struct FocusManager {
    pub order: Vec<String>,
    pub index: usize,
}

impl FocusManager {
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            index: 0,
        }
    }

    pub fn from_ids(ids: Vec<String>) -> Self {
        Self {
            order: ids,
            index: 0,
        }
    }

    pub fn focus_next(&mut self, hosts: &HashMap<String, PanelHost>) {
        if self.order.is_empty() {
            return;
        }
        self.unfocus_current(hosts);
        self.index = (self.index + 1) % self.order.len();
        self.focus_current(hosts);
    }

    pub fn focus_prev(&mut self, hosts: &HashMap<String, PanelHost>) {
        if self.order.is_empty() {
            return;
        }
        self.unfocus_current(hosts);
        self.index = if self.index == 0 {
            self.order.len() - 1
        } else {
            self.index - 1
        };
        self.focus_current(hosts);
    }

    pub fn focused_panel_id(&self) -> Option<&str> {
        self.order.get(self.index).map(|s| s.as_str())
    }

    pub fn focus_order_index(&self, panel_id: &str) -> Option<usize> {
        self.order.iter().position(|s| s == panel_id)
    }

    pub fn set_focus_index(&mut self, idx: usize, hosts: &HashMap<String, PanelHost>) {
        self.unfocus_current(hosts);
        self.index = idx.min(self.order.len().saturating_sub(1));
        self.focus_current(hosts);
    }

    /// Rebuild focus order from layout panel IDs. Clamp index if needed.
    pub fn rebuild(&mut self, ids: Vec<String>) {
        self.order = ids;
        if self.index >= self.order.len() && !self.order.is_empty() {
            self.index = self.order.len() - 1;
        }
    }

    /// Focus the first panel in the order.
    pub fn focus_first(&mut self, hosts: &HashMap<String, PanelHost>) {
        self.index = 0;
        self.focus_current(hosts);
    }

    /// Focus the current panel (public, for use after index changes).
    pub fn focus_current_pub(&self, hosts: &HashMap<String, PanelHost>) {
        self.focus_current(hosts);
    }

    fn unfocus_current(&self, hosts: &HashMap<String, PanelHost>) {
        if let Some(current) = self.order.get(self.index) {
            if let Some(host) = hosts.get(current) {
                host.set_focused(false);
            }
        }
    }

    fn focus_current(&self, hosts: &HashMap<String, PanelHost>) {
        if let Some(next) = self.order.get(self.index) {
            if let Some(host) = hosts.get(next) {
                host.set_focused(true);
            }
        }
    }
}
