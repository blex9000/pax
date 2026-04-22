//! Shared Markdown-to-TextBuffer renderer.
//!
//! Used by both the standalone Markdown panel (`panels::markdown`) and the
//! Code Editor's Markdown tab (`panels::editor::markdown_view`). A hand-rolled
//! parser — deliberately minimal — with GTK `TextTag`s doing the visual work.

use gtk4::prelude::*;

// Code block backgrounds — slight contrast against each theme family's main
// surface, without overriding the default text foreground (so GTK keeps
// contrast for us as the theme changes).
const CODE_BG_DARK: &str = "#1a1a1a";
const CODE_BG_LIGHT: &str = "#ececec";

pub(crate) fn render_markdown_to_view(tv: &gtk4::TextView, content: &str) {
    let buf = tv.buffer();
    buf.set_text("");
    let tt = buf.tag_table();

    // Theme-reactive colors for code blocks and inline code. The dark theme's
    // #2a2a2a block background becomes unreadable against default dark text
    // on a light theme; pick contrasting colors based on the active theme.
    let is_light = matches!(
        crate::theme::current_theme().color_scheme(),
        libadwaita::ColorScheme::ForceLight
    );
    let code_bg = if is_light { CODE_BG_LIGHT } else { CODE_BG_DARK };

    // `ensure` re-applies the callback every time so theme-reactive tags
    // (code, code_block) update when the renderer runs again after a theme
    // change, not just on first creation.
    let ensure = |name: &str, f: &dyn Fn(&gtk4::TextTag)| {
        let t = if let Some(t) = tt.lookup(name) {
            t
        } else {
            let t = gtk4::TextTag::new(Some(name));
            tt.add(&t);
            t
        };
        f(&t);
    };
    ensure("h1", &|t| {
        t.set_size_points(20.0);
        t.set_weight(700);
    });
    ensure("h2", &|t| {
        t.set_size_points(16.0);
        t.set_weight(700);
    });
    ensure("h3", &|t| {
        t.set_size_points(14.0);
        t.set_weight(700);
    });
    ensure("bold", &|t| {
        t.set_weight(700);
    });
    ensure("italic", &|t| {
        t.set_style(gtk4::pango::Style::Italic);
    });
    ensure("strike", &|t| {
        t.set_strikethrough(true);
    });
    ensure("code", &|t| {
        t.set_family(Some("monospace"));
        // No background — inline code stays as-is to avoid visually heavy
        // spans; only the code_block (fenced) paragraph gets a background.
    });
    ensure("code_block", &|t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some(code_bg));
        // Foreground intentionally unset: inheriting the theme's default
        // text color keeps readability on light and dark themes alike.
        t.set_left_margin(20);
    });
    ensure("link", &|t| {
        t.set_foreground(Some("#5588ff"));
        t.set_underline(gtk4::pango::Underline::Single);
    });
    ensure("bullet", &|t| {
        t.set_left_margin(20);
    });
    ensure("bq", &|t| {
        t.set_left_margin(20);
        t.set_style(gtk4::pango::Style::Italic);
        t.set_foreground(Some("#888888"));
    });
    ensure("sep", &|t| {
        t.set_foreground(Some("#666666"));
        t.set_size_points(6.0);
    });

    let mut it = buf.end_iter();
    let mut in_code = false;
    for line in content.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
            let hint = line.trim_start_matches('`').trim();
            if in_code && !hint.is_empty() {
                buf.insert_with_tags_by_name(&mut it, &format!("─── {} ───\n", hint), &["sep"]);
            } else if !in_code {
                buf.insert_with_tags_by_name(&mut it, "───────\n", &["sep"]);
            }
            continue;
        }
        if in_code {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", line), &["code_block"]);
            continue;
        }
        if line.starts_with("### ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[4..]), &["h3"]);
        } else if line.starts_with("## ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[3..]), &["h2"]);
        } else if line.starts_with("# ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[2..]), &["h1"]);
        } else if line.starts_with("---") || line.starts_with("***") {
            buf.insert_with_tags_by_name(&mut it, "────────────────────\n", &["sep"]);
        } else if line.starts_with("- ") || line.starts_with("* ") {
            buf.insert_with_tags_by_name(&mut it, &format!("  • {}\n", &line[2..]), &["bullet"]);
        } else if line.starts_with("> ") {
            buf.insert_with_tags_by_name(&mut it, &format!("│ {}\n", &line[2..]), &["bq"]);
        } else {
            render_inline(&buf, &mut it, line);
            buf.insert(&mut it, "\n");
        }
    }
}

fn render_inline(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, text: &str) {
    let c: Vec<char> = text.chars().collect();
    let n = c.len();
    let mut i = 0;
    let mut p = String::new();
    while i < n {
        if i + 1 < n && c[i] == '*' && c[i + 1] == '*' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 2;
            let s = i;
            while i + 1 < n && !(c[i] == '*' && c[i + 1] == '*') {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["bold"]);
            if i + 1 < n {
                i += 2;
            }
        } else if c[i] == '*' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != '*' {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["italic"]);
            if i < n {
                i += 1;
            }
        } else if c[i] == '`' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != '`' {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["code"]);
            if i < n {
                i += 1;
            }
        } else if c[i] == '[' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != ']' {
                i += 1;
            }
            let lt: String = c[s..i].iter().collect();
            if i + 1 < n && c[i] == ']' && c[i + 1] == '(' {
                i += 2;
                while i < n && c[i] != ')' {
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
            } else if i < n {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &lt, &["link"]);
        } else {
            p.push(c[i]);
            i += 1;
        }
    }
    if !p.is_empty() {
        buf.insert(it, &p);
    }
}
