use gtk4::{gio, prelude::*};
#[cfg(feature = "vte")]
use vte4::prelude::*;

const DEFAULT_EXPORT_NAME: &str = "terminal-output.txt";

pub(super) fn save_bytes_dialog(anchor: &gtk4::Widget, bytes: Vec<u8>) {
    let dialog = gtk4::FileDialog::builder()
        .title("Save Terminal Output")
        .modal(true)
        .initial_name(DEFAULT_EXPORT_NAME)
        .build();

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some("Text files"));
    filter.add_pattern("*.txt");
    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));

    let parent = parent_window(anchor);
    let anchor = anchor.clone();
    dialog.save(parent.as_ref(), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        if let Err(err) = file.replace_contents(
            &bytes,
            None,
            false,
            gio::FileCreateFlags::REPLACE_DESTINATION,
            gio::Cancellable::NONE,
        ) {
            tracing::warn!("terminal export: failed to save output: {}", err);
            show_error(&anchor, "Could not save terminal output", &err.to_string());
        }
    });
}

#[cfg(feature = "vte")]
pub(super) fn vte_output_snapshot(vte: &vte4::Terminal) -> Result<Vec<u8>, gtk4::glib::Error> {
    let stream = gio::MemoryOutputStream::new_resizable();
    vte.write_contents_sync(&stream, vte4::WriteFlags::Default, gio::Cancellable::NONE)?;
    stream.close(gio::Cancellable::NONE)?;
    Ok(stream.steal_as_bytes().as_ref().to_vec())
}

pub(super) fn show_error(anchor: &gtk4::Widget, message: &str, detail: &str) {
    let dialog = gtk4::AlertDialog::builder()
        .modal(true)
        .message(message)
        .detail(detail)
        .buttons(["OK"])
        .default_button(0)
        .cancel_button(0)
        .build();
    let parent = parent_window(anchor);
    dialog.choose(parent.as_ref(), gio::Cancellable::NONE, |_| {});
}

fn parent_window(anchor: &gtk4::Widget) -> Option<gtk4::Window> {
    anchor
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok())
}
