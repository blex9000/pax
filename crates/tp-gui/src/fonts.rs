use std::ffi::CString;
use std::path::PathBuf;

/// Register bundled fonts (resources/fonts/) so they are available to both
/// CSS (via GTK) and Pango (terminal font descriptions) regardless of whether
/// the font is installed system-wide.
///
/// On Linux this uses fontconfig (`FcConfigAppFontAddDir`).
/// On macOS Pango goes through CoreText, which doesn't read fontconfig, so we
/// also register each font file via `CTFontManagerRegisterFontsForURL`.
pub fn register_bundled_fonts() {
    for dir in font_search_paths() {
        if dir.is_dir() {
            register_fontconfig_dir(&dir);
            #[cfg(target_os = "macos")]
            macos::register_dir_via_core_text(&dir);
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

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::path::Path;

    type CFTypeRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFURLRef = *const c_void;
    type CFErrorRef = *const c_void;
    type CFIndex = isize;
    type Boolean = u8;
    type CFStringEncoding = u32;
    type CTFontManagerScope = u32;

    const KCF_ALLOCATOR_DEFAULT: CFAllocatorRef = std::ptr::null();
    const KCF_STRING_ENCODING_UTF8: CFStringEncoding = 0x0800_0100;
    const KCF_URL_POSIX_PATH_STYLE: u32 = 0;
    /// `kCTFontManagerScopeProcess` — only this process sees the registered
    /// fonts; nothing is written to the user's Font Book.
    const KCT_FONT_MANAGER_SCOPE_PROCESS: CTFontManagerScope = 1;

    #[link(name = "CoreFoundation", kind = "framework")]
    #[link(name = "CoreText", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithBytes(
            alloc: CFAllocatorRef,
            bytes: *const u8,
            num_bytes: CFIndex,
            encoding: CFStringEncoding,
            is_external_representation: Boolean,
        ) -> CFStringRef;
        fn CFURLCreateWithFileSystemPath(
            allocator: CFAllocatorRef,
            file_path: CFStringRef,
            path_style: u32,
            is_directory: Boolean,
        ) -> CFURLRef;
        fn CFRelease(cf: CFTypeRef);
        fn CTFontManagerRegisterFontsForURL(
            font_url: CFURLRef,
            scope: CTFontManagerScope,
            error: *mut CFErrorRef,
        ) -> Boolean;
    }

    pub fn register_dir_via_core_text(dir: &Path) {
        // CoreText's CFURL needs a filesystem path; relative paths work in
        // principle but depend on the current process cwd, which varies
        // between cargo run (repo root) and a .app bundle launch (/). Make
        // the path absolute so the registration is unambiguous.
        let canonical = match dir.canonicalize() {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(
                    "core-text font registration: cannot canonicalize {}: {}",
                    dir.display(),
                    err
                );
                return;
            }
        };
        let Ok(entries) = std::fs::read_dir(&canonical) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_font = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    let ext = ext.to_ascii_lowercase();
                    matches!(ext.as_str(), "ttf" | "otf" | "ttc")
                });
            if !is_font {
                continue;
            }
            register_one_file(&path);
        }
    }

    fn register_one_file(path: &Path) {
        let path_str = path.to_string_lossy();
        let bytes = path_str.as_bytes();
        tracing::debug!("core-text: registering font {}", path.display());
        unsafe {
            let cf_path = CFStringCreateWithBytes(
                KCF_ALLOCATOR_DEFAULT,
                bytes.as_ptr(),
                bytes.len() as CFIndex,
                KCF_STRING_ENCODING_UTF8,
                0,
            );
            if cf_path.is_null() {
                return;
            }
            let cf_url = CFURLCreateWithFileSystemPath(
                KCF_ALLOCATOR_DEFAULT,
                cf_path,
                KCF_URL_POSIX_PATH_STYLE,
                0,
            );
            CFRelease(cf_path);
            if cf_url.is_null() {
                return;
            }
            let mut error: CFErrorRef = std::ptr::null();
            let ok = CTFontManagerRegisterFontsForURL(
                cf_url,
                KCT_FONT_MANAGER_SCOPE_PROCESS,
                &mut error,
            );
            CFRelease(cf_url);
            // ok == 0 with a non-null error can mean "font already registered"
            // from a previous run in the same login session, which is fine —
            // the existing registration still satisfies Pango lookups. We log
            // at debug level so users don't see spurious warnings on every
            // relaunch, but the message is available via RUST_LOG=debug.
            if ok == 0 {
                tracing::debug!(
                    "core-text: CTFontManagerRegisterFontsForURL returned false for {} (likely already registered)",
                    path.display()
                );
            }
            if !error.is_null() {
                CFRelease(error);
            }
        }
    }
}
