use ratatui::layout::Rect;
use tp_core::workspace::LayoutNode;

/// Resolved layout: maps panel IDs to screen rectangles.
#[derive(Debug, Default)]
pub struct ResolvedLayout {
    pub panels: Vec<(String, Rect)>,
}

impl ResolvedLayout {
    /// Resolve a LayoutNode tree into panel ID → Rect mappings.
    pub fn resolve(node: &LayoutNode, area: Rect) -> Self {
        let mut resolved = Self::default();
        resolve_node(node, area, &mut resolved);
        resolved
    }

    /// Get the rect for a specific panel.
    pub fn get(&self, panel_id: &str) -> Option<Rect> {
        self.panels
            .iter()
            .find(|(id, _)| id == panel_id)
            .map(|(_, r)| *r)
    }

    /// Get all panel IDs in layout order.
    pub fn panel_ids(&self) -> Vec<&str> {
        self.panels.iter().map(|(id, _)| id.as_str()).collect()
    }
}

fn resolve_node(node: &LayoutNode, area: Rect, out: &mut ResolvedLayout) {
    match node {
        LayoutNode::Panel { id } => {
            out.panels.push((id.clone(), area));
        }
        LayoutNode::Hsplit { children, ratios } => {
            let rects = split_horizontal(area, ratios, children.len());
            for (child, rect) in children.iter().zip(rects) {
                resolve_node(child, rect, out);
            }
        }
        LayoutNode::Vsplit { children, ratios } => {
            let rects = split_vertical(area, ratios, children.len());
            for (child, rect) in children.iter().zip(rects) {
                resolve_node(child, rect, out);
            }
        }
        LayoutNode::Tabs { children, .. } => {
            // For tabs, only the first child is visible by default
            // (tab switching is handled at the app level)
            if let Some(first) = children.first() {
                // Reserve 1 row for tab bar
                if area.height > 1 {
                    let content = Rect {
                        x: area.x,
                        y: area.y + 1,
                        width: area.width,
                        height: area.height - 1,
                    };
                    resolve_node(first, content, out);
                }
            }
        }
    }
}

fn split_horizontal(area: Rect, ratios: &[f64], count: usize) -> Vec<Rect> {
    let ratios = normalize_ratios(ratios, count);
    let total_width = area.width as f64;
    let mut rects = Vec::new();
    let mut x = area.x;

    for (i, ratio) in ratios.iter().enumerate() {
        let w = if i == ratios.len() - 1 {
            // Last panel gets remaining space to avoid rounding gaps
            area.x + area.width - x
        } else {
            (total_width * ratio).round() as u16
        };
        rects.push(Rect {
            x,
            y: area.y,
            width: w,
            height: area.height,
        });
        x += w;
    }

    rects
}

fn split_vertical(area: Rect, ratios: &[f64], count: usize) -> Vec<Rect> {
    let ratios = normalize_ratios(ratios, count);
    let total_height = area.height as f64;
    let mut rects = Vec::new();
    let mut y = area.y;

    for (i, ratio) in ratios.iter().enumerate() {
        let h = if i == ratios.len() - 1 {
            area.y + area.height - y
        } else {
            (total_height * ratio).round() as u16
        };
        rects.push(Rect {
            x: area.x,
            y,
            width: area.width,
            height: h,
        });
        y += h;
    }

    rects
}

fn normalize_ratios(ratios: &[f64], count: usize) -> Vec<f64> {
    let mut r: Vec<f64> = if ratios.len() == count {
        ratios.to_vec()
    } else {
        vec![1.0; count]
    };
    let sum: f64 = r.iter().sum();
    if sum > 0.0 {
        for v in &mut r {
            *v /= sum;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_hsplit() {
        let node = LayoutNode::Hsplit {
            children: vec![
                LayoutNode::Panel { id: "p1".into() },
                LayoutNode::Panel { id: "p2".into() },
            ],
            ratios: vec![1.0, 1.0],
        };
        let area = Rect::new(0, 0, 100, 50);
        let resolved = ResolvedLayout::resolve(&node, area);
        assert_eq!(resolved.panels.len(), 2);
        assert_eq!(resolved.panels[0].1.width, 50);
        assert_eq!(resolved.panels[1].1.width, 50);
        assert_eq!(resolved.panels[1].1.x, 50);
    }

    #[test]
    fn test_resolve_nested() {
        let node = LayoutNode::Vsplit {
            children: vec![
                LayoutNode::Hsplit {
                    children: vec![
                        LayoutNode::Panel { id: "p1".into() },
                        LayoutNode::Panel { id: "p2".into() },
                    ],
                    ratios: vec![1.0, 1.0],
                },
                LayoutNode::Panel { id: "p3".into() },
            ],
            ratios: vec![1.0, 1.0],
        };
        let area = Rect::new(0, 0, 100, 50);
        let resolved = ResolvedLayout::resolve(&node, area);
        assert_eq!(resolved.panels.len(), 3);
    }
}
