//! Markdown → PDF export via `gtk4::PrintOperation`, block-based.
//!
//! v1 used one big `pango::Layout` for the whole document. That broke
//! down on three fronts:
//!   - Pango `<span background>` colours only glyph cells, so code-block
//!     and table backgrounds looked like striped text rather than solid
//!     rectangles.
//!   - A single layout has one `WrapMode`. Tables can't sit at NoWrap
//!     while paragraphs WordWrap.
//!   - Pango paginates by lines, not by logical blocks, so tables and
//!     diagrams got chopped across page boundaries.
//!
//! v2: parse markdown into a `Vec<Block>`. Each block knows how to
//! measure itself (height at a given page width) and how to draw
//! itself onto a Cairo context at a given Y offset — including its
//! own background rectangle when relevant. The paginator packs blocks
//! greedily, starting a new page whenever the next block doesn't fit
//! whole. Backgrounds are clean rects, tables don't wrap, blocks
//! don't get chopped.

use gtk4::prelude::*;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

const CODE_BG: (f64, f64, f64) = (0.93, 0.93, 0.93);
const BLOCK_GAP_PT: f64 = 4.0;
const CODE_PAD_PT: f64 = 6.0;
const FOOTER_HEIGHT_PT: f64 = 14.0;
const MONO_BASE_PT: f64 = 9.0;
const MONO_MIN_PT: f64 = 5.0;

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
    let blocks: Rc<Vec<Block>> = Rc::new(markdown_to_blocks(markdown));
    // pages[i] = (block_index_start, block_index_end_exclusive)
    let pages: Rc<RefCell<Vec<(usize, usize)>>> = Rc::new(RefCell::new(Vec::new()));

    let print_op = gtk4::PrintOperation::new();
    print_op.set_export_filename(output_path);
    print_op.set_unit(gtk4::Unit::Points);

    let setup = gtk4::PageSetup::new();
    setup.set_paper_size(&gtk4::PaperSize::new(Some("iso_a4")));
    setup.set_top_margin(20.0, gtk4::Unit::Mm);
    setup.set_bottom_margin(20.0, gtk4::Unit::Mm);
    setup.set_left_margin(20.0, gtk4::Unit::Mm);
    setup.set_right_margin(20.0, gtk4::Unit::Mm);
    print_op.set_default_page_setup(Some(&setup));

    {
        let blocks = blocks.clone();
        let pages = pages.clone();
        print_op.connect_begin_print(move |op, ctx| {
            let page_width = ctx.width();
            let page_height = ctx.height();
            let usable_height = page_height - FOOTER_HEIGHT_PT;

            let mut paginated: Vec<(usize, usize)> = Vec::new();
            let mut cur_start = 0usize;
            let mut cur_height = 0.0_f64;
            for (i, block) in blocks.iter().enumerate() {
                let h = block.measure(ctx, page_width) + BLOCK_GAP_PT;
                let block_taller_than_page = h > usable_height;
                let fits = cur_height + h <= usable_height;
                if !fits && i > cur_start && !block_taller_than_page {
                    paginated.push((cur_start, i));
                    cur_start = i;
                    cur_height = h;
                } else {
                    cur_height += h;
                }
            }
            if cur_start < blocks.len() {
                paginated.push((cur_start, blocks.len()));
            }
            if paginated.is_empty() {
                paginated.push((0, 0));
            }
            *pages.borrow_mut() = paginated.clone();
            op.set_n_pages(paginated.len() as i32);
        });
    }

    {
        let blocks = blocks.clone();
        let pages = pages.clone();
        print_op.connect_draw_page(move |_, ctx, page_no| {
            let cr = ctx.cairo_context();
            let page_width = ctx.width();
            let page_height = ctx.height();

            let pages_ref = pages.borrow();
            let total_pages = pages_ref.len();
            let (start, end) = pages_ref
                .get(page_no as usize)
                .copied()
                .unwrap_or((0, 0));

            let mut y = 0.0_f64;
            for i in start..end {
                let block = &blocks[i];
                block.draw(ctx, &cr, 0.0, y, page_width);
                y += block.measure(ctx, page_width) + BLOCK_GAP_PT;
            }

            draw_footer(ctx, &cr, page_no as usize + 1, total_pages, page_width, page_height);
        });
    }

    let _ = print_op.run(gtk4::PrintOperationAction::Export, Some(parent));
}

// ────────────────────────────────────────────────────────────────────
// Block model
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Block {
    /// Heading at a given level. `markup` is Pango markup of inline
    /// content (no outer span wrapper — `draw` adds size/weight).
    Heading {
        level: HeadingLevel,
        markup: String,
    },
    /// Paragraph of inline markup.
    Paragraph { markup: String },
    /// Code block: monospace, light grey rectangular background, no
    /// wrapping.
    CodeBlock { text: String },
    /// Block quote: italic, indented.
    BlockQuote { markup: String },
    /// List items, each as inline markup. `ordered` switches between
    /// numeric and bullet markers.
    List {
        items: Vec<String>,
        ordered: bool,
        start: u64,
    },
    /// GFM table — rendered as a single monospace block with box-drawing
    /// borders (matches the on-screen panel) and a clean rectangular bg.
    Table {
        rows: Vec<Vec<String>>,
        body_start: usize,
    },
    /// Horizontal rule.
    Rule,
}

impl Block {
    fn measure(&self, ctx: &gtk4::PrintContext, page_width: f64) -> f64 {
        match self {
            Block::Heading { level, markup } => {
                let layout = ctx.create_pango_layout();
                let desc = heading_font(*level);
                layout.set_font_description(Some(&desc));
                layout.set_width((page_width * gtk4::pango::SCALE as f64) as i32);
                layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                layout.set_markup(markup);
                pu_to_pt(layout.size().1)
            }
            Block::Paragraph { markup } | Block::BlockQuote { markup } => {
                let layout = ctx.create_pango_layout();
                let desc = body_font();
                layout.set_font_description(Some(&desc));
                layout.set_width((page_width * gtk4::pango::SCALE as f64) as i32);
                layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                layout.set_markup(markup);
                pu_to_pt(layout.size().1)
            }
            Block::CodeBlock { text } => {
                let layout = code_layout(ctx, text, page_width);
                pu_to_pt(layout.size().1) + 2.0 * CODE_PAD_PT
            }
            Block::List { items, ordered, start } => {
                let mut total = 0.0;
                for (i, item) in items.iter().enumerate() {
                    let bullet = if *ordered {
                        format!("{}. ", *start + i as u64)
                    } else {
                        "• ".to_string()
                    };
                    let layout = ctx.create_pango_layout();
                    let desc = body_font();
                    layout.set_font_description(Some(&desc));
                    layout.set_width((page_width * gtk4::pango::SCALE as f64) as i32);
                    layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                    layout.set_markup(&format!("{}{}", escape(&bullet), item));
                    total += pu_to_pt(layout.size().1);
                }
                total
            }
            Block::Table { rows, body_start } => {
                let text = render_table_monospace(rows, *body_start);
                let layout = code_layout(ctx, &text, page_width);
                pu_to_pt(layout.size().1)
            }
            Block::Rule => 8.0,
        }
    }

    fn draw(
        &self,
        ctx: &gtk4::PrintContext,
        cr: &gtk4::cairo::Context,
        x: f64,
        y: f64,
        page_width: f64,
    ) {
        match self {
            Block::Heading { level, markup } => {
                let layout = ctx.create_pango_layout();
                layout.set_font_description(Some(&heading_font(*level)));
                layout.set_width((page_width * gtk4::pango::SCALE as f64) as i32);
                layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                layout.set_markup(markup);
                cr.move_to(x, y);
                pangocairo::functions::show_layout(cr, &layout);
            }
            Block::Paragraph { markup } => {
                let layout = ctx.create_pango_layout();
                layout.set_font_description(Some(&body_font()));
                layout.set_width((page_width * gtk4::pango::SCALE as f64) as i32);
                layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                layout.set_markup(markup);
                cr.move_to(x, y);
                pangocairo::functions::show_layout(cr, &layout);
            }
            Block::BlockQuote { markup } => {
                let indent = 18.0;
                let layout = ctx.create_pango_layout();
                layout.set_font_description(Some(&body_font()));
                layout.set_width(((page_width - indent) * gtk4::pango::SCALE as f64) as i32);
                layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                layout.set_markup(&format!("<i>{}</i>", markup));
                let h_pt = pu_to_pt(layout.size().1);
                cr.save().ok();
                {
                    cr.set_source_rgb(0.6, 0.6, 0.6);
                    cr.set_line_width(2.0);
                    cr.move_to(x + 4.0, y + 2.0);
                    cr.line_to(x + 4.0, y + h_pt - 2.0);
                    cr.stroke().ok();
                }
                cr.restore().ok();
                cr.move_to(x + indent, y);
                pangocairo::functions::show_layout(cr, &layout);
            }
            Block::CodeBlock { text } => {
                let layout = code_layout(ctx, text, page_width);
                let h_pt = pu_to_pt(layout.size().1) + 2.0 * CODE_PAD_PT;
                cr.save().ok();
                {
                    cr.set_source_rgb(CODE_BG.0, CODE_BG.1, CODE_BG.2);
                    cr.rectangle(x, y, page_width, h_pt);
                    cr.fill().ok();
                }
                cr.restore().ok();
                cr.move_to(x + CODE_PAD_PT, y + CODE_PAD_PT);
                pangocairo::functions::show_layout(cr, &layout);
            }
            Block::List { items, ordered, start } => {
                let mut cy = y;
                for (i, item) in items.iter().enumerate() {
                    let bullet = if *ordered {
                        format!("{}. ", *start + i as u64)
                    } else {
                        "• ".to_string()
                    };
                    let layout = ctx.create_pango_layout();
                    layout.set_font_description(Some(&body_font()));
                    layout.set_width((page_width * gtk4::pango::SCALE as f64) as i32);
                    layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                    layout.set_markup(&format!("{}{}", escape(&bullet), item));
                    cr.move_to(x, cy);
                    pangocairo::functions::show_layout(cr, &layout);
                    cy += pu_to_pt(layout.size().1);
                }
            }
            Block::Table { rows, body_start } => {
                // No background rectangle for tables — the box-drawing
                // characters already give the table a visual frame.
                let text = render_table_monospace(rows, *body_start);
                let layout = code_layout(ctx, &text, page_width);
                cr.move_to(x, y);
                pangocairo::functions::show_layout(cr, &layout);
            }
            Block::Rule => {
                cr.save().ok();
                {
                    cr.set_source_rgb(0.7, 0.7, 0.7);
                    cr.set_line_width(0.5);
                    cr.move_to(x, y + 4.0);
                    cr.line_to(x + page_width, y + 4.0);
                    cr.stroke().ok();
                }
                cr.restore().ok();
            }
        }
    }
}

/// Build a monospace, no-wrap Pango layout for `text`. If the natural
/// width at the base font size exceeds `max_width_pt`, shrink the font
/// proportionally so the block stays visible inside the page (down to
/// MONO_MIN_PT). Used for code blocks and tables.
fn code_layout(ctx: &gtk4::PrintContext, text: &str, max_width_pt: f64) -> gtk4::pango::Layout {
    let layout = ctx.create_pango_layout();
    layout.set_wrap(gtk4::pango::WrapMode::WordChar);
    layout.set_width(-1);
    layout.set_text(text);

    // First measurement: at the base monospace size.
    layout.set_font_description(Some(&mono_font()));
    let natural_pt = pu_to_pt(layout.size().0);
    if natural_pt > max_width_pt && natural_pt > 0.0 && max_width_pt > 0.0 {
        let scale = max_width_pt / natural_pt;
        let scaled_size = (MONO_BASE_PT * scale).max(MONO_MIN_PT);
        let scaled = gtk4::pango::FontDescription::from_string(&format!(
            "Monospace {}",
            format_pt(scaled_size)
        ));
        layout.set_font_description(Some(&scaled));
    }
    layout
}

fn format_pt(pt: f64) -> String {
    // Pango parses both "9" and "9.5" as a font size — use one
    // decimal so we don't lose precision after the auto-shrink ratio.
    format!("{:.1}", pt)
}

fn draw_footer(
    ctx: &gtk4::PrintContext,
    cr: &gtk4::cairo::Context,
    page_no: usize,
    total_pages: usize,
    page_width: f64,
    page_height: f64,
) {
    let layout = ctx.create_pango_layout();
    layout.set_alignment(gtk4::pango::Alignment::Center);
    layout.set_width((page_width * gtk4::pango::SCALE as f64) as i32);
    let desc = gtk4::pango::FontDescription::from_string("Sans 9");
    layout.set_font_description(Some(&desc));
    layout.set_text(&format!("{} / {}", page_no, total_pages));

    let h_pt = pu_to_pt(layout.size().1);
    cr.save().ok();
    cr.set_source_rgb(0.55, 0.55, 0.55);
    cr.move_to(0.0, page_height - h_pt - 2.0);
    pangocairo::functions::show_layout(cr, &layout);
    cr.restore().ok();
}

fn pu_to_pt(pu: i32) -> f64 {
    pu as f64 / gtk4::pango::SCALE as f64
}

fn heading_font(level: HeadingLevel) -> gtk4::pango::FontDescription {
    let size = match level {
        HeadingLevel::H1 => 18,
        HeadingLevel::H2 => 16,
        HeadingLevel::H3 => 14,
        HeadingLevel::H4 => 12,
        HeadingLevel::H5 => 11,
        HeadingLevel::H6 => 10,
    };
    gtk4::pango::FontDescription::from_string(&format!("Sans Bold {}", size))
}

fn body_font() -> gtk4::pango::FontDescription {
    gtk4::pango::FontDescription::from_string("Sans 10")
}

fn mono_font() -> gtk4::pango::FontDescription {
    gtk4::pango::FontDescription::from_string("Monospace 9")
}

// ────────────────────────────────────────────────────────────────────
// Markdown → blocks
// ────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct BlockBuilder {
    blocks: Vec<Block>,
    /// Accumulator for the current inline markup (paragraph / heading
    /// content / list-item / table-cell / blockquote).
    inline_buf: String,
    /// Where the current `inline_buf` should be flushed to.
    inline_target: InlineTarget,
    /// Stack of list contexts (ordered, start).
    lists: Vec<(bool, u64)>,
    /// Items collected for the innermost open list.
    list_stack: Vec<Vec<String>>,
    /// Table state.
    table_rows: Vec<Vec<String>>,
    table_body_start: usize,
    table_row: Vec<String>,
    /// Code block accumulator.
    code_buf: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineTarget {
    None,
    Paragraph,
    Heading(HeadingLevel),
    BlockQuote,
    ListItem,
    TableCell,
    CodeBlock,
}

impl Default for InlineTarget {
    fn default() -> Self {
        InlineTarget::None
    }
}

fn markdown_to_blocks(content: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(content, opts);

    let mut b = BlockBuilder::default();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                b.inline_buf.clear();
                b.inline_target = InlineTarget::Heading(level);
            }
            Event::End(TagEnd::Heading(level)) => {
                let markup = std::mem::take(&mut b.inline_buf);
                b.blocks.push(Block::Heading { level, markup });
                b.inline_target = InlineTarget::None;
                let _ = level;
            }
            Event::Start(Tag::Paragraph) => {
                b.inline_buf.clear();
                b.inline_target = InlineTarget::Paragraph;
            }
            Event::End(TagEnd::Paragraph) => {
                let markup = std::mem::take(&mut b.inline_buf);
                b.blocks.push(Block::Paragraph { markup });
                b.inline_target = InlineTarget::None;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                b.inline_buf.clear();
                b.inline_target = InlineTarget::BlockQuote;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                let markup = std::mem::take(&mut b.inline_buf);
                b.blocks.push(Block::BlockQuote { markup });
                b.inline_target = InlineTarget::None;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                b.code_buf.clear();
                b.inline_target = InlineTarget::CodeBlock;
            }
            Event::End(TagEnd::CodeBlock) => {
                let text = std::mem::take(&mut b.code_buf);
                // Trim a single trailing newline pulldown-cmark adds.
                let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
                b.blocks.push(Block::CodeBlock { text });
                b.inline_target = InlineTarget::None;
            }
            Event::Start(Tag::List(start)) => {
                b.lists.push((start.is_some(), start.unwrap_or(1)));
                b.list_stack.push(Vec::new());
            }
            Event::End(TagEnd::List(_)) => {
                let items = b.list_stack.pop().unwrap_or_default();
                let (ordered, start) = b.lists.pop().unwrap_or((false, 1));
                b.blocks.push(Block::List {
                    items,
                    ordered,
                    start,
                });
            }
            Event::Start(Tag::Item) => {
                b.inline_buf.clear();
                b.inline_target = InlineTarget::ListItem;
            }
            Event::End(TagEnd::Item) => {
                let markup = std::mem::take(&mut b.inline_buf);
                if let Some(items) = b.list_stack.last_mut() {
                    items.push(markup);
                }
                b.inline_target = InlineTarget::None;
            }
            Event::Start(Tag::Table(_)) => {
                b.table_rows.clear();
                b.table_body_start = 0;
                b.table_row.clear();
            }
            Event::End(TagEnd::Table) => {
                let rows = std::mem::take(&mut b.table_rows);
                b.blocks.push(Block::Table {
                    rows,
                    body_start: b.table_body_start,
                });
                b.table_body_start = 0;
            }
            Event::Start(Tag::TableHead) => {
                // pulldown-cmark emits the header cells as direct
                // children of TableHead (no enclosing TableRow), so
                // we manage the row buffer ourselves here.
                b.table_row.clear();
            }
            Event::End(TagEnd::TableHead) => {
                if !b.table_row.is_empty() {
                    b.table_rows.push(std::mem::take(&mut b.table_row));
                }
                b.table_body_start = b.table_rows.len();
            }
            Event::Start(Tag::TableRow) => {
                b.table_row.clear();
            }
            Event::End(TagEnd::TableRow) => {
                b.table_rows.push(std::mem::take(&mut b.table_row));
            }
            Event::Start(Tag::TableCell) => {
                b.inline_buf.clear();
                b.inline_target = InlineTarget::TableCell;
            }
            Event::End(TagEnd::TableCell) => {
                let cell = std::mem::take(&mut b.inline_buf);
                b.table_row.push(cell);
                b.inline_target = InlineTarget::None;
            }
            Event::Start(Tag::Strong) => append_markup(&mut b, "<b>"),
            Event::End(TagEnd::Strong) => append_markup(&mut b, "</b>"),
            Event::Start(Tag::Emphasis) => append_markup(&mut b, "<i>"),
            Event::End(TagEnd::Emphasis) => append_markup(&mut b, "</i>"),
            Event::Start(Tag::Strikethrough) => append_markup(&mut b, "<s>"),
            Event::End(TagEnd::Strikethrough) => append_markup(&mut b, "</s>"),
            Event::Code(s) => match b.inline_target {
                InlineTarget::TableCell => b.inline_buf.push_str(&s),
                _ => {
                    append_markup(&mut b, "<tt>");
                    append_markup(&mut b, &escape(&s));
                    append_markup(&mut b, "</tt>");
                }
            },
            Event::Text(t) => match b.inline_target {
                InlineTarget::CodeBlock => b.code_buf.push_str(&t),
                InlineTarget::TableCell => b.inline_buf.push_str(&t),
                _ => b.inline_buf.push_str(&escape(&t)),
            },
            Event::SoftBreak => append_markup(&mut b, " "),
            Event::HardBreak => append_markup(&mut b, "\n"),
            Event::Rule => b.blocks.push(Block::Rule),
            Event::Start(Tag::Link { .. }) => {}
            Event::End(TagEnd::Link) => {}
            Event::Start(Tag::Image { .. }) => append_markup(&mut b, "[image]"),
            Event::End(TagEnd::Image) => {}
            _ => {}
        }
    }

    b.blocks
}

/// Append a Pango-markup fragment (e.g. `<b>`, `<i>`) to the current
/// inline buffer. Drops the fragment entirely when the buffer feeds a
/// destination that's rendered as plain monospace text — code blocks
/// and table cells — since markup tags would print as literal `<b>`
/// rather than apply formatting.
fn append_markup(b: &mut BlockBuilder, s: &str) {
    if matches!(
        b.inline_target,
        InlineTarget::None | InlineTarget::CodeBlock | InlineTarget::TableCell
    ) {
        return;
    }
    b.inline_buf.push_str(s);
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

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
    // Strip the very last newline so the layout doesn't leave an empty
    // line at the bottom of the rendered block.
    out.strip_suffix('\n').unwrap_or(&out).to_string()
}

pub fn suggested_pdf_name(source_path: &Path) -> String {
    let stem = source_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".to_string());
    format!("{}.pdf", stem)
}
