use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoopRunInMode};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::NSDictionary;
use std::env::current_exe;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

/// Return the name of the current focused application.
///
/// # Example
///
/// ```
/// use create::backend::macos::current_focus_app_name;
///
/// println!("{}", current_focus_app_name()); // Output: "Code"
/// ```
pub fn current_focus_app_name() -> String {
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, false as _);
    }

    let workspace = { NSWorkspace::sharedWorkspace() };

    if let Some(app) = { workspace.frontmostApplication() } {
        if let Some(name) = { app.localizedName() } {
            return name.to_string();
        }
    }

    "Unknown".to_string()
}

/// Return the path of the current focused application.
///
/// # Example
///
/// ```
/// use create::backend::macos::current_focus_app_path;
///
/// println!("{:?}", current_focus_app_path()); // Output: "/Applications/Visual Studio Code.app"
/// ```
pub fn current_focus_app_path() -> PathBuf {
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, false as _);
    }

    let workspace = { NSWorkspace::sharedWorkspace() };

    let path_string = {
        workspace
            .frontmostApplication()
            .and_then(|app| app.bundleURL())
            .and_then(|url| url.path())
    };

    if let Some(ns_string) = path_string {
        return PathBuf::from(ns_string.to_string());
    }

    PathBuf::new()
}

/// Return the icon file path of the current focused application.
///
/// - Icon will be saved as a PNG file.
/// - Icon file name will be the same as the app name.
///
/// # Example
///
/// ```
/// use create::backend::macos::current_focus_app_icon_path;
///
/// println!("{:?}", current_focus_app_icon_path()); // Output: "/Applications/THIS.app/Contents/MacOS/Code.png"
/// ```
pub fn current_focus_app_icon_path() -> PathBuf {
    let current_focus_app_name = current_focus_app_name();
    let current_focus_app_icon_path = current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("{}.png", current_focus_app_name));

    if !current_focus_app_icon_path.exists() {
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, false as _);
        }

        let workspace = NSWorkspace::sharedWorkspace();

        if let Some(app) = workspace.frontmostApplication() {
            if let Some(icon) = app.icon() {
                if let Some(tiff_data) = icon.TIFFRepresentation() {
                    if let Some(bitmap_rep) = NSBitmapImageRep::imageRepWithData(&tiff_data) {
                        if let Some(png_data) = unsafe {
                            bitmap_rep.representationUsingType_properties(
                                NSBitmapImageFileType::PNG,
                                &NSDictionary::new(),
                            )
                        } {
                            if let Ok(mut file) = File::create(&current_focus_app_icon_path) {
                                file.write_all(unsafe { png_data.as_bytes_unchecked() })
                                    .unwrap();
                            }
                        }
                    }
                }
            }
        }
    }

    debug_assert!(
        current_focus_app_icon_path.exists(),
        "Icon file should exist after called."
    );

    current_focus_app_icon_path
}
