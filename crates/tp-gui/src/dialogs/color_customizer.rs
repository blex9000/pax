use gtk4::prelude::*;

/// Open the color customizer dialog for tweaking individual theme CSS tokens.
pub fn show_color_customizer_dialog(parent: &impl IsA<gtk4::Window>) {
    let dialog = gtk4::Window::builder()
        .title("Customize Theme Colors")
        .transient_for(parent)
        .modal(true)
        .default_width(480)
        .default_height(600)
        .build();
    crate::theme::configure_dialog_window(&dialog);

    let label = gtk4::Label::new(Some("Color customizer — coming soon"));
    label.set_margin_top(24);
    label.set_margin_bottom(24);
    label.set_margin_start(24);
    label.set_margin_end(24);
    dialog.set_child(Some(&label));
    dialog.present();
}
