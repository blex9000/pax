use std::collections::VecDeque;

use tp_core::alert::{self, CompiledAlert};
use tp_core::workspace::AlertAction;

/// Ring buffer that stores terminal output lines and scans for alerts.
pub struct OutputBuffer {
    lines: VecDeque<String>,
    max_lines: usize,
    /// Partial line accumulator (data before \n).
    partial: String,
}

/// Alert triggered on a specific line.
#[derive(Debug)]
pub struct TriggeredAlert {
    pub line: String,
    pub actions: Vec<AlertAction>,
}

impl OutputBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(max_lines),
            max_lines,
            partial: String::new(),
        }
    }

    /// Feed raw output bytes and return any triggered alerts.
    pub fn feed(
        &mut self,
        data: &[u8],
        panel_id: &str,
        panel_groups: &[String],
        alerts: &[CompiledAlert],
    ) -> Vec<TriggeredAlert> {
        let text = String::from_utf8_lossy(data);
        let mut triggered = Vec::new();

        for ch in text.chars() {
            if ch == '\n' {
                let line = std::mem::take(&mut self.partial);

                // Scan for alerts
                let actions: Vec<AlertAction> =
                    alert::scan_line(alerts, &line, panel_id, panel_groups)
                        .into_iter()
                        .cloned()
                        .collect();

                if !actions.is_empty() {
                    triggered.push(TriggeredAlert {
                        line: line.clone(),
                        actions,
                    });
                }

                // Store in ring buffer
                if self.lines.len() >= self.max_lines {
                    self.lines.pop_front();
                }
                self.lines.push_back(line);
            } else {
                self.partial.push(ch);
            }
        }

        triggered
    }

    /// Get all stored lines.
    pub fn lines(&self) -> &VecDeque<String> {
        &self.lines
    }

    /// Get the last N lines.
    pub fn tail(&self, n: usize) -> Vec<&str> {
        self.lines
            .iter()
            .rev()
            .take(n)
            .rev()
            .map(|s| s.as_str())
            .collect()
    }

    /// Clear all stored output.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.partial.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer() {
        let mut buf = OutputBuffer::new(3);
        buf.feed(b"line1\nline2\nline3\nline4\n", "p1", &[], &[]);
        assert_eq!(buf.lines().len(), 3);
        assert_eq!(buf.lines()[0], "line2");
        assert_eq!(buf.lines()[2], "line4");
    }

    #[test]
    fn test_partial_lines() {
        let mut buf = OutputBuffer::new(10);
        buf.feed(b"hel", "p1", &[], &[]);
        buf.feed(b"lo\nworld\n", "p1", &[], &[]);
        assert_eq!(buf.lines().len(), 2);
        assert_eq!(buf.lines()[0], "hello");
        assert_eq!(buf.lines()[1], "world");
    }
}
