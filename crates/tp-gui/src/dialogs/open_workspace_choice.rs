//! Modal prompt: "open workspace X — in this window, in a new window,
//! or cancel?". Shown when the user clicks a scheduled-alert toast
//! whose note lives in a different workspace than the one currently
//! on screen.

use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita as adw;

/// Caller's choice.
pub enum OpenWorkspaceDecision {
    /// Replace the current workspace with this one.
    ThisWindow(PathBuf),
    /// Spawn a fresh Pax process pointing at this workspace.
    NewWindow(PathBuf),
    /// User dismissed the dialog.
    Cancel,
}

/// Show the dialog. `on_decision` fires once with the user's choice.
pub fn show(
    parent: &adw::ApplicationWindow,
    workspace_name: &str,
    workspace_path: &std::path::Path,
    on_decision: Rc<dyn Fn(OpenWorkspaceDecision)>,
) {
    let dialog = gtk4::Window::builder()
        .title("Open workspace")
        .transient_for(parent)
        .modal(true)
        .default_width(420)
        .build();
    crate::theme::configure_dialog_window(&dialog);

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(16);
    root.set_margin_end(16);

    let heading = gtk4::Label::new(Some(&format!(
        "Open workspace \u{201C}{}\u{201D}?",
        workspace_name
    )));
    heading.add_css_class("title-4");
    heading.set_halign(gtk4::Align::Start);
    heading.set_wrap(true);
    heading.set_xalign(0.0);
    root.append(&heading);

    let path_label = gtk4::Label::new(Some(&workspace_path.display().to_string()));
    path_label.add_css_class("dim-label");
    path_label.add_css_class("caption");
    path_label.set_halign(gtk4::Align::Start);
    path_label.set_xalign(0.0);
    path_label.set_wrap(true);
    path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
    root.append(&path_label);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_top(6);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let new_window_btn = gtk4::Button::with_label("Open in new window");
    let this_window_btn = gtk4::Button::with_label("Open in this window");
    this_window_btn.add_css_class("suggested-action");

    btn_row.append(&cancel_btn);
    btn_row.append(&new_window_btn);
    btn_row.append(&this_window_btn);
    root.append(&btn_row);

    dialog.set_child(Some(&root));

    let path = workspace_path.to_path_buf();

    {
        let d = dialog.clone();
        let on_decision = on_decision.clone();
        cancel_btn.connect_clicked(move |_| {
            d.close();
            on_decision(OpenWorkspaceDecision::Cancel);
        });
    }
    {
        let d = dialog.clone();
        let on_decision = on_decision.clone();
        let path = path.clone();
        new_window_btn.connect_clicked(move |_| {
            d.close();
            on_decision(OpenWorkspaceDecision::NewWindow(path.clone()));
        });
    }
    {
        let d = dialog.clone();
        let on_decision = on_decision.clone();
        let path = path.clone();
        this_window_btn.connect_clicked(move |_| {
            d.close();
            on_decision(OpenWorkspaceDecision::ThisWindow(path.clone()));
        });
    }

    dialog.present();
}
