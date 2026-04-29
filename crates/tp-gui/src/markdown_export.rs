//! Markdown → PDF export via `gtk4::PrintOperation`.
//!
//! Two-stage pipeline:
//!   1. `markdown_to_markup` walks a pulldown-cmark stream and emits
//!      Pango markup (subset of HTML-ish tags Pango understands).
//!   2. `run_print` builds a single `pango::Layout` for the markup,
//!      lets `PrintOperation` paginate it onto an A4 surface, and writes
//!      the output as a PDF via `set_export_filename`.
//!
//! Scope is intentionally narrow for v1: headings, paragraphs,
//! bold/italic/inline-code, code blocks (monospace, no syntax
//! highlighting), bullet/numbered lists, block quotes (italic),
//! basic tables (rendered the same way the screen renderer does — box
//! drawing). Links print as plain text, images are skipped, notebook
//! cells render only their markdown body.

use gtk4::prelude::*;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Public entry point used by the markdown panel and the editor's
/// markdown tab. Opens a Save dialog and, if the user confirms,
/// runs the print operation in `Export` mode (no print dialog).
pub fn export_markdown_to_pdf(parent: &gtk4::Window, content: &str, suggested_name: &str) {
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Esporta PDF");
    dialog.set_initial_name(Some(suggested_name));
    let parent_for_dialog = parent.clone();
    let parent_for_print = parent.clone();
    let content = content.to_string();
    dialog.save(
        Some(&parent_for_dialog),
        None::<&gtk4::gio::Cancellable>,
        move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            run_print(&parent_for_print, &content, &path);
        },
    );
}

fn run_print(parent: &gtk4::Window, markdown: &str, output_path: &Path) {
    let print_op = gtk4::PrintOperation::new();
    print_op.set_export_filename(output_path);
    print_op.set_unit(gtk4::Unit::Points);

    // A4 paper, 20mm margins on every side. Same default as Save-As-PDF
    // in most desktop apps; user-customisable later if needed.
    let setup = gtk4::PageSetup::new();
    let paper = gtk4::PaperSize::new(Some("iso_a4"));
    setup.set_paper_size(&paper);
    setup.set_top_margin(20.0, gtk4::Unit::Mm);
    setup.set_bottom_margin(20.0, gtk4::Unit::Mm);
    setup.set_left_margin(20.0, gtk4::Unit::Mm);
    setup.set_right_margin(20.0, gtk4::Unit::Mm);
    print_op.set_default_page_setup(Some(&setup));

    let markup = markdown_to_markup(markdown);
    let layout_holder: Rc<RefCell<Option<gtk4::pango::Layout>>> = Rc::new(RefCell::new(None));
    let page_height_pt = Rc::new(Cell::new(0.0_f64));

    {
        let layout_holder = layout_holder.clone();
        let page_height_pt = page_height_pt.clone();
        let markup = markup.clone();
        print_op.connect_begin_print(move |op, ctx| {
            let layout = ctx.create_pango_layout();
            let width_pt = ctx.width();
            let height_pt = ctx.height();
            page_height_pt.set(height_pt);

            layout.set_width((width_pt * gtk4::pango::SCALE as f64) as i32);
            layout.set_wrap(gtk4::pango::WrapMode::WordChar);
            layout.set_markup(&markup);

            let total_height_pu = layout.size().1 as f64;
            let total_height_pt = total_height_pu / gtk4::pango::SCALE as f64;
            let pages = ((total_height_pt / height_pt).ceil() as i32).max(1);
            op.set_n_pages(pages);
            *layout_holder.borrow_mut() = Some(layout);
        });
    }

    {
        let layout_holder = layout_holder.clone();
        let page_height_pt = page_height_pt.clone();
        print_op.connect_draw_page(move |_, ctx, page_no| {
            let cr = ctx.cairo_context();
            let layout_opt = layout_holder.borrow();
            let Some(layout) = layout_opt.as_ref() else { return };
            // Paint the full layout on every page, translating up by
            // `page_no * page_height` so each page shows its slice.
            // The print context already clips to the page's printable
            // area, so off-page content is dropped automatically.
            cr.translate(0.0, -(page_no as f64) * page_height_pt.get());
            pangocairo::functions::show_layout(&cr, layout);
        });
    }

    let _ = print_op.run(gtk4::PrintOperationAction::Export, Some(parent));
}

#[derive(Default)]
struct ConvState {
    /// Stack of list contexts: (is_ordered, next_number).
    lists: Vec<(bool, u64)>,
    in_code_block: bool,
    in_table: bool,
    table_header_done: bool,
    /// Accumulator for the current table row's cells.
    table_row: Vec<String>,
    /// All collected rows so the renderer can compute column widths.
    table_rows: Vec<Vec<String>>,
    /// Index where the body starts (after the header row, if any).
    table_body_start: usize,
    /// Buffer for the cell currently being built (so inline events
    /// inside a cell accumulate before the cell ends).
    cell_buf: String,
    in_cell: bool,
}

/// Convert a CommonMark + GFM string to Pango markup.
fn markdown_to_markup(content: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(content, opts);

    let mut out = String::new();
    let mut state = ConvState::default();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                push(&mut out, &state, &heading_open(level));
            }
            Event::End(TagEnd::Heading(_)) => {
                push(&mut out, &state, "</span>\n\n");
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                push(&mut out, &state, "\n\n");
            }
            Event::Start(Tag::Strong) => push(&mut out, &state, "<b>"),
            Event::End(TagEnd::Strong) => push(&mut out, &state, "</b>"),
            Event::Start(Tag::Emphasis) => push(&mut out, &state, "<i>"),
            Event::End(TagEnd::Emphasis) => push(&mut out, &state, "</i>"),
            Event::Start(Tag::Strikethrough) => push(&mut out, &state, "<s>"),
            Event::End(TagEnd::Strikethrough) => push(&mut out, &state, "</s>"),
            Event::Code(s) => {
                push(&mut out, &state, "<tt>");
                push(&mut out, &state, &escape(&s));
                push(&mut out, &state, "</tt>");
            }
            Event::Start(Tag::CodeBlock(_)) => {
                state.in_code_block = true;
                out.push_str("<tt><span background=\"#eeeeee\">");
            }
            Event::End(TagEnd::CodeBlock) => {
                state.in_code_block = false;
                out.push_str("</span></tt>\n");
            }
            Event::Start(Tag::BlockQuote(_)) => {
                push(&mut out, &state, "<i>");
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                push(&mut out, &state, "</i>\n");
            }
            Event::Start(Tag::List(start)) => {
                state.lists.push((start.is_some(), start.unwrap_or(1)));
            }
            Event::End(TagEnd::List(_)) => {
                state.lists.pop();
                push(&mut out, &state, "\n");
            }
            Event::Start(Tag::Item) => {
                let depth = state.lists.len().saturating_sub(1);
                let indent: String = std::iter::repeat("  ").take(depth).collect();
                let bullet = if let Some((ordered, next)) = state.lists.last_mut() {
                    if *ordered {
                        let s = format!("{}{}. ", indent, next);
                        *next += 1;
                        s
                    } else {
                        format!("{}• ", indent)
                    }
                } else {
                    String::new()
                };
                push(&mut out, &state, &bullet);
            }
            Event::End(TagEnd::Item) => {
                push(&mut out, &state, "\n");
            }
            Event::Start(Tag::Link { .. }) => {
                // Ignored: link text passes through as plain text.
            }
            Event::End(TagEnd::Link) => {}
            Event::Start(Tag::Image { .. }) => {
                // No image embedding in v1; just emit a `[image]` marker.
                push(&mut out, &state, "[image]");
            }
            Event::End(TagEnd::Image) => {}
            Event::Start(Tag::Table(_)) => {
                state.in_table = true;
                state.table_header_done = false;
                state.table_rows.clear();
                state.table_body_start = 0;
            }
            Event::End(TagEnd::Table) => {
                state.in_table = false;
                let table_text = render_table_monospace(&state.table_rows, state.table_body_start);
                out.push_str("<tt><span background=\"#eeeeee\">");
                out.push_str(&escape(&table_text));
                out.push_str("</span></tt>\n");
            }
            Event::Start(Tag::TableHead) => {}
            Event::End(TagEnd::TableHead) => {
                state.table_body_start = state.table_rows.len();
                state.table_header_done = true;
            }
            Event::Start(Tag::TableRow) => {
                state.table_row.clear();
            }
            Event::End(TagEnd::TableRow) => {
                state.table_rows.push(std::mem::take(&mut state.table_row));
            }
            Event::Start(Tag::TableCell) => {
                state.cell_buf.clear();
                state.in_cell = true;
            }
            Event::End(TagEnd::TableCell) => {
                state.in_cell = false;
                let cell = std::mem::take(&mut state.cell_buf);
                state.table_row.push(cell);
            }
            Event::Text(t) => {
                if state.in_cell {
                    state.cell_buf.push_str(&t);
                } else {
                    push(&mut out, &state, &escape(&t));
                }
            }
            Event::SoftBreak => {
                if state.in_cell {
                    state.cell_buf.push(' ');
                } else {
                    push(&mut out, &state, " ");
                }
            }
            Event::HardBreak => {
                if state.in_cell {
                    state.cell_buf.push(' ');
                } else {
                    push(&mut out, &state, "\n");
                }
            }
            Event::Rule => {
                push(&mut out, &state, "\n— — —\n\n");
            }
            _ => {}
        }
    }

    out
}

fn push(out: &mut String, state: &ConvState, s: &str) {
    if state.in_table {
        return;
    }
    out.push_str(s);
}

fn heading_open(level: HeadingLevel) -> String {
    let size = match level {
        HeadingLevel::H1 => 22000,
        HeadingLevel::H2 => 18000,
        HeadingLevel::H3 => 15000,
        HeadingLevel::H4 => 13000,
        HeadingLevel::H5 => 12000,
        HeadingLevel::H6 => 11000,
    };
    format!("<span size=\"{}\" weight=\"bold\">", size)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render a parsed table as a single monospace block with box-drawing
/// borders. Mirrors `markdown_render::render_table` so the PDF looks
/// like the on-screen panel, and uses Unicode display width so wide
/// glyphs (CJK / emoji) line up.
fn render_table_monospace(rows: &[Vec<String>], body_start: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if rows.is_empty() {
        return String::new();
    }
    let n_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if n_cols == 0 {
        return String::new();
    }
    let mut widths = vec![0_usize; n_cols];
    for row in rows {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(cell.width());
        }
    }
    let format_row = |row: &[String]| -> String {
        let mut out = String::from("│ ");
        for c in 0..n_cols {
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            let pad = widths[c].saturating_sub(cell.width());
            out.push_str(cell);
            out.push_str(&" ".repeat(pad));
            out.push_str(if c + 1 == n_cols { " │" } else { " │ " });
        }
        out.push('\n');
        out
    };
    let border = |l: char, m: char, r: char| {
        let mut s = String::new();
        s.push(l);
        for (c, w) in widths.iter().enumerate() {
            s.push_str(&"─".repeat(w + 2));
            s.push(if c + 1 == n_cols { r } else { m });
        }
        s.push('\n');
        s
    };
    let mut out = border('┌', '┬', '┐');
    for (idx, row) in rows.iter().enumerate() {
        out.push_str(&format_row(row));
        if idx + 1 == body_start && body_start > 0 {
            out.push_str(&border('├', '┼', '┤'));
        }
    }
    out.push_str(&border('└', '┴', '┘'));
    out
}

/// Suggest a default PDF filename next to the source markdown file.
pub fn suggested_pdf_name(source_path: &Path) -> String {
    let stem = source_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".to_string());
    format!("{}.pdf", stem)
}

/// Resolve the directory the Save dialog should default to: the
/// source file's parent, or the current working directory.
#[allow(dead_code)]
pub fn suggested_pdf_dir(source_path: &Path) -> PathBuf {
    source_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}
