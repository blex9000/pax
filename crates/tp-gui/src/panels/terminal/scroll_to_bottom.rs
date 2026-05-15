use gtk4::prelude::*;

pub(super) fn button() -> gtk4::Button {
    let button = gtk4::Button::builder()
        .icon_name("go-down-symbolic")
        .focus_on_click(false)
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::End)
        .tooltip_text("Scroll to bottom")
        .build();
    button.add_css_class("terminal-scroll-bottom");
    button.set_margin_bottom(12);
    button.set_margin_end(12);
    button.set_visible(false);
    button
}
