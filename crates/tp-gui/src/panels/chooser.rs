use gtk4::prelude::*;
use std::rc::Rc;

use super::registry::PanelRegistry;
use super::PanelBackend;

/// Callback when the user selects a panel type.
pub type OnTypeChosen = Rc<dyn Fn(&str, &str)>; // (panel_id, type_id)

/// Empty panel that shows a type chooser.
/// Displayed when a new panel is created without a type.
#[derive(Debug)]
pub struct ChooserPanel {
    container: gtk4::Box,
    widget: gtk4::Widget,
}

impl ChooserPanel {
    pub fn new(panel_id: &str, registry: &PanelRegistry, on_chosen: Option<OnTypeChosen>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_valign(gtk4::Align::Center);
        container.set_halign(gtk4::Align::Center);
        container.set_spacing(16);

        // Title
        let title = gtk4::Label::new(Some("Set panel type"));
        title.add_css_class("title-2");
        container.append(&title);

        let subtitle = gtk4::Label::new(Some("Choose what this panel should display"));
        subtitle.add_css_class("dim-label");
        container.append(&subtitle);

        // Grid of type buttons
        let flow = gtk4::FlowBox::new();
        flow.set_max_children_per_line(3);
        flow.set_min_children_per_line(2);
        flow.set_selection_mode(gtk4::SelectionMode::None);
        flow.set_homogeneous(true);
        flow.set_column_spacing(8);
        flow.set_row_spacing(8);
        flow.set_halign(gtk4::Align::Center);

        for type_info in registry.types() {
            let btn = gtk4::Button::new();
            btn.add_css_class("flat");
            btn.add_css_class("card");
            btn.add_css_class("panel-type-btn");

            let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            vbox.set_margin_top(12);
            vbox.set_margin_bottom(12);
            vbox.set_margin_start(16);
            vbox.set_margin_end(16);

            let icon = gtk4::Image::from_icon_name(&type_info.icon);
            icon.set_pixel_size(32);
            vbox.append(&icon);

            let name = gtk4::Label::new(Some(&type_info.display_name));
            name.add_css_class("heading");
            vbox.append(&name);

            let desc = gtk4::Label::new(Some(&type_info.description));
            desc.add_css_class("dim-label");
            desc.add_css_class("caption");
            desc.set_wrap(true);
            desc.set_max_width_chars(20);
            vbox.append(&desc);

            btn.set_child(Some(&vbox));

            let type_id = type_info.id.clone();
            let pid = panel_id.to_string();
            let cb = on_chosen.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref callback) = cb {
                    callback(&pid, &type_id);
                }
            });

            flow.insert(&btn, -1);
        }

        container.append(&flow);

        let widget = container.clone().upcast::<gtk4::Widget>();

        Self { container, widget }
    }
}

impl PanelBackend for ChooserPanel {
    fn panel_type(&self) -> &str {
        "chooser"
    }

    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    fn on_focus(&self) {
        self.container.grab_focus();
    }
}
