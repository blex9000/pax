use std::ffi::CString;
use std::path::PathBuf;

/// Register bundled fonts (resources/fonts/) with fontconfig so they are
/// available to both CSS (via GTK) and Pango (terminal font descriptions)
/// regardless of whether the font is installed system-wide.
pub fn register_bundled_fonts() {
    for dir in font_search_paths() {
        if dir.is_dir() {
            register_fontconfig_dir(&dir);
        }
    }
}

fn font_search_paths() -> Vec<PathBuf> {
    let exe = std::env::current_exe().ok();

    let mut paths = vec![
        PathBuf::from("resources/fonts"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/fonts"),
    ];

    // macOS .app bundle: <bundle>/Contents/Resources/fonts
    if let Some(ref exe) = exe {
        if let Some(bundle_res) = exe.parent().and_then(|p| p.parent()).map(|p| p.join("Resources/fonts")) {
            paths.push(bundle_res);
        }
        // Flat install next to binary: <dir>/../resources/fonts
        if let Some(flat) = exe.parent().and_then(|p| p.parent()).map(|p| p.join("resources/fonts")) {
            paths.push(flat);
        }
        // AppImage / Linux FHS-style install: <dir>/../share/fonts/pax
        // (bundled by scripts/build-appimage.sh into $APPDIR/usr/share/fonts/pax)
        if let Some(fhs) = exe.parent().and_then(|p| p.parent()).map(|p| p.join("share/fonts/pax")) {
            paths.push(fhs);
        }
    }

    paths
}

fn register_fontconfig_dir(dir: &std::path::Path) {
    let Ok(cpath) = CString::new(dir.to_string_lossy().as_bytes().to_vec()) else {
        return;
    };

    unsafe {
        let config = FcConfigGetCurrent();
        if !config.is_null() {
            FcConfigAppFontAddDir(config, cpath.as_ptr() as *const u8);
        }
    }
}

// Minimal fontconfig FFI — only what we need.
#[link(name = "fontconfig")]
extern "C" {
    fn FcConfigGetCurrent() -> *mut std::ffi::c_void;
    fn FcConfigAppFontAddDir(config: *mut std::ffi::c_void, dir: *const u8) -> i32;
}
