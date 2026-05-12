use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use super::file_backend::FileBackend;
use super::overview_ruler::{build_overview_ruler, OverviewRulerKind};
use super::text_context_menu;
use super::text_shortcuts::{install_text_clipboard_shortcuts, install_text_history_shortcuts};

/// Side-by-side diff core returned by `build_diff_view_parts`. Callers wrap
/// this in their own header + Ctrl+S handler.
pub(super) struct DiffViewParts {
    /// Horizontal Paned: left column (old, read-only) | right column (new, editable),
    /// each with its own overview ruler and synced vertical scrolling.
    pub(super) paned: gtk4::Paned,
    /// Right-side editable buffer. Callers read its text on save.
    pub(super) new_buf: sourceview5::Buffer,
}

/// Build the side-by-side diff core: two SourceView columns (left old/read-only,
/// right new/editable) inside a horizontal Paned, with synced vertical
/// scrolling and per-side overview rulers marking changed lines. Used by both
/// the git-diff view (HEAD vs working) and the merge view (disk vs unsaved
/// buffer); each caller wraps this body in its own header and Ctrl+S handler.
pub(super) fn build_diff_view_parts(
    file_path: &Path,
    old_content: &str,
    new_content: &str,
) -> DiffViewParts {
    let old_display = super::text_content::displayable_gtk_text(old_content);
    let new_display = super::text_content::displayable_gtk_text(new_content);
    let old_content = old_display.as_ref();
    let new_content = new_display.as_ref();

    let old_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    old_buf.set_text(old_content);
    old_buf.set_highlight_syntax(true);
    let new_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    new_buf.set_text(new_content);
    new_buf.set_highlight_syntax(true);

    let lang_manager = sourceview5::LanguageManager::default();
    if let Some(lang) = lang_manager.guess_language(Some(file_path), None::<&str>) {
        old_buf.set_language(Some(&lang));
        new_buf.set_language(Some(&lang));
    }
    crate::theme::register_sourceview_buffer(&old_buf);
    crate::theme::register_sourceview_buffer(&new_buf);

    let mut old_change_lines: Vec<i32> = Vec::new();
    let mut new_change_lines: Vec<i32> = Vec::new();
    {
        let diff = similar::TextDiff::from_lines(old_content, new_content);

        let ensure_diff_tags = |buf: &sourceview5::Buffer| {
            let tt = buf.tag_table();
            if tt.lookup("diff-del").is_none() {
                let tag = gtk4::TextTag::new(Some("diff-del"));
                tag.set_paragraph_background(Some("rgba(220, 50, 47, 0.25)"));
                tt.add(&tag);
            }
            if tt.lookup("diff-add").is_none() {
                let tag = gtk4::TextTag::new(Some("diff-add"));
                tag.set_paragraph_background(Some("rgba(40, 180, 60, 0.25)"));
                tt.add(&tag);
            }
        };
        ensure_diff_tags(&old_buf);
        ensure_diff_tags(&new_buf);

        let mut old_line = 0i32;
        let mut new_line = 0i32;
        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Equal => {
                    old_line += 1;
                    new_line += 1;
                }
                similar::ChangeTag::Delete => {
                    if let Some(start) = old_buf.iter_at_line(old_line) {
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        end.forward_char();
                        old_buf.apply_tag_by_name("diff-del", &start, &end);
                    }
                    old_change_lines.push(old_line);
                    old_line += 1;
                }
                similar::ChangeTag::Insert => {
                    if let Some(start) = new_buf.iter_at_line(new_line) {
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        end.forward_char();
                        new_buf.apply_tag_by_name("diff-add", &start, &end);
                    }
                    new_change_lines.push(new_line);
                    new_line += 1;
                }
            }
        }
    }

    let file_path_owned = file_path.to_path_buf();
    let make_sv =
        |buf: &sourceview5::Buffer, editable: bool| -> (sourceview5::View, gtk4::ScrolledWindow) {
            let view = sourceview5::View::with_buffer(buf);
            view.add_css_class("editor-code-view");
            view.set_editable(editable);
            view.set_show_line_numbers(true);
            view.set_monospace(true);
            view.set_left_margin(3);
            install_text_clipboard_shortcuts(&view);
            install_text_history_shortcuts(&view);
            if editable {
                view.set_auto_indent(true);
                view.set_tab_width(4);
            }
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_child(Some(&view));
            scroll.set_vexpand(true);
            scroll.set_hexpand(true);
            let file_path_factory = file_path_owned.clone();
            let buf_factory = buf.clone();
            text_context_menu::install(&scroll, &view, editable, move |_click_line| {
                if !editable {
                    return Vec::new();
                }
                text_context_menu::format_item_for(&file_path_factory, &buf_factory)
                    .map(|i| vec![i])
                    .unwrap_or_default()
            });
            (view, scroll)
        };

    let (old_view, old_scroll) = make_sv(&old_buf, false);
    let (new_view, new_scroll) = make_sv(&new_buf, true);

    let old_bar = build_overview_ruler(
        old_change_lines,
        old_buf.line_count(),
        OverviewRulerKind::Delete,
        &old_view,
    );
    let new_bar = build_overview_ruler(
        new_change_lines,
        new_buf.line_count(),
        OverviewRulerKind::Insert,
        &new_view,
    );
    // Rulers on the outer edges of the diff so the Paned separator between
    // old/new scrollviews stays grabable. The outer-left ruler also gets a
    // generous start margin so it clears the *main* sidebar paned's resize
    // grab zone (which extends further than the visible handle).
    old_bar.set_margin_start(12);
    let old_column = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    old_column.append(&old_bar);
    old_column.append(&old_scroll);
    let new_column = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    new_column.append(&new_scroll);
    new_column.append(&new_bar);

    let syncing = Rc::new(Cell::new(false));
    {
        let ns = new_scroll.clone();
        let s = syncing.clone();
        old_scroll.vadjustment().connect_value_changed(move |adj| {
            if !s.get() {
                s.set(true);
                ns.vadjustment().set_value(adj.value());
                s.set(false);
            }
        });
    }
    {
        let os = old_scroll.clone();
        let s = syncing.clone();
        new_scroll.vadjustment().connect_value_changed(move |adj| {
            if !s.get() {
                s.set(true);
                os.vadjustment().set_value(adj.value());
                s.set(false);
            }
        });
    }

    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_start_child(Some(&old_column));
    paned.set_end_child(Some(&new_column));

    DiffViewParts { paned, new_buf }
}

/// Show a side-by-side diff for a single file within a commit.
pub(super) fn show_commit_file_diff(
    content_stack: &gtk4::Stack,
    _notebook: &gtk4::Notebook,
    commit_hash: &str,
    file_rel: &str,
    backend: Arc<dyn FileBackend>,
) {
    // Get old version (parent commit) and new version (this commit)
    let parent = format!("{}~1", commit_hash);
    let old_content = backend
        .git_show(&format!("{}:{}", parent, file_rel))
        .unwrap_or_default();
    let new_content = backend
        .git_show(&format!("{}:{}", commit_hash, file_rel))
        .unwrap_or_default();
    let old_display = super::text_content::displayable_gtk_text(&old_content);
    let new_display = super::text_content::displayable_gtk_text(&new_content);
    let old_content = old_display.as_ref();
    let new_content = new_display.as_ref();

    let old_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    old_buf.set_text(old_content);
    old_buf.set_highlight_syntax(true);
    let new_buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    new_buf.set_text(new_content);
    new_buf.set_highlight_syntax(true);

    // Syntax highlighting
    let lang_manager = sourceview5::LanguageManager::default();
    let file_path = Path::new(file_rel);
    if let Some(lang) = lang_manager.guess_language(Some(file_path), None::<&str>) {
        old_buf.set_language(Some(&lang));
        new_buf.set_language(Some(&lang));
    }
    crate::theme::register_sourceview_buffer(&old_buf);
    crate::theme::register_sourceview_buffer(&new_buf);

    // Highlight diff
    {
        let diff = similar::TextDiff::from_lines(old_content, new_content);
        let ensure_tags = |buf: &sourceview5::Buffer| {
            let tt = buf.tag_table();
            if tt.lookup("diff-del").is_none() {
                let tag = gtk4::TextTag::new(Some("diff-del"));
                tag.set_paragraph_background(Some("rgba(220, 50, 47, 0.25)"));
                tt.add(&tag);
            }
            if tt.lookup("diff-add").is_none() {
                let tag = gtk4::TextTag::new(Some("diff-add"));
                tag.set_paragraph_background(Some("rgba(40, 180, 60, 0.25)"));
                tt.add(&tag);
            }
        };
        ensure_tags(&old_buf);
        ensure_tags(&new_buf);

        let mut old_line = 0i32;
        let mut new_line = 0i32;
        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Equal => {
                    old_line += 1;
                    new_line += 1;
                }
                similar::ChangeTag::Delete => {
                    if let Some(start) = old_buf.iter_at_line(old_line) {
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        end.forward_char();
                        old_buf.apply_tag_by_name("diff-del", &start, &end);
                    }
                    old_line += 1;
                }
                similar::ChangeTag::Insert => {
                    if let Some(start) = new_buf.iter_at_line(new_line) {
                        let mut end = start.clone();
                        end.forward_to_line_end();
                        end.forward_char();
                        new_buf.apply_tag_by_name("diff-add", &start, &end);
                    }
                    new_line += 1;
                }
            }
        }
    }

    let make_sv = |buf: &sourceview5::Buffer| -> gtk4::ScrolledWindow {
        let view = sourceview5::View::with_buffer(buf);
        view.add_css_class("editor-code-view");
        view.set_editable(false);
        view.set_show_line_numbers(true);
        view.set_monospace(true);
        view.set_left_margin(3);
        install_text_clipboard_shortcuts(&view);
        install_text_history_shortcuts(&view);
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_child(Some(&view));
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        text_context_menu::install(&scroll, &view, false, |_click_line| Vec::new());
        scroll
    };

    let old_scroll = make_sv(&old_buf);
    let new_scroll = make_sv(&new_buf);

    // Sync scrolling
    let syncing = Rc::new(Cell::new(false));
    {
        let ns = new_scroll.clone();
        let s = syncing.clone();
        old_scroll.vadjustment().connect_value_changed(move |adj| {
            if !s.get() {
                s.set(true);
                ns.vadjustment().set_value(adj.value());
                s.set(false);
            }
        });
    }
    {
        let os = old_scroll.clone();
        let s = syncing;
        new_scroll.vadjustment().connect_value_changed(move |adj| {
            if !s.get() {
                s.set(true);
                os.vadjustment().set_value(adj.value());
                s.set(false);
            }
        });
    }

    // Build diff view
    let diff_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    diff_box.set_vexpand(true);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.set_margin_start(8);
    header.set_margin_end(8);
    header.set_margin_top(4);
    header.set_margin_bottom(4);

    let back_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    back_btn.add_css_class("flat");
    back_btn.set_tooltip_text(Some("Back to commit"));
    header.append(&back_btn);

    let file_label = gtk4::Label::new(Some(&format!(
        "{}  {} → {}",
        file_rel,
        &parent[..parent.len().min(8)],
        &commit_hash[..commit_hash.len().min(8)]
    )));
    file_label.add_css_class("heading");
    file_label.set_hexpand(true);
    file_label.set_halign(gtk4::Align::Start);
    header.append(&file_label);

    // Revert this file to before this commit
    let revert_btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
    revert_btn.add_css_class("flat");
    revert_btn.set_tooltip_text(Some("Revert this file to before this commit"));
    {
        let be = backend.clone();
        let parent_c = parent.clone();
        let fp = file_rel.to_string();
        let cs = content_stack.clone();
        revert_btn.connect_clicked(move |_| {
            let _ = be.git_command(&["checkout", &parent_c, "--", &fp]);
            cs.set_visible_child_name("commit-diff");
        });
    }
    header.append(&revert_btn);

    diff_box.append(&header);

    // Column labels
    let labels = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    let old_label = gtk4::Label::new(Some(&format!(
        "← PREVIOUS  {}  ({})",
        file_rel,
        &parent[..parent.len().min(8)]
    )));
    old_label.add_css_class("dim-label");
    old_label.set_hexpand(true);
    old_label.set_margin_start(8);
    let new_label = gtk4::Label::new(Some(&format!(
        "CURRENT  {}  ({}) →",
        file_rel,
        &commit_hash[..commit_hash.len().min(8)]
    )));
    new_label.add_css_class("dim-label");
    new_label.set_hexpand(true);
    new_label.set_margin_start(8);
    labels.append(&old_label);
    labels.append(&new_label);
    diff_box.append(&labels);

    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_start_child(Some(&old_scroll));
    paned.set_end_child(Some(&new_scroll));
    diff_box.append(&paned);

    // Replace content
    if let Some(old) = content_stack.child_by_name("commit-file-diff") {
        content_stack.remove(&old);
    }
    content_stack.add_named(&diff_box, Some("commit-file-diff"));
    content_stack.set_visible_child_name("commit-file-diff");

    // Back goes to commit-diff view
    {
        let cs = content_stack.clone();
        back_btn.connect_clicked(move |_| {
            cs.set_visible_child_name("commit-diff");
        });
    }
}
