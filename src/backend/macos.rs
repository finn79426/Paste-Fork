use core_foundation::base::TCFType;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoopRunInMode};
use core_foundation::string::CFString;
use objc::runtime::{Class, Object};
use objc::{msg_send, sel, sel_impl};
use objc2::rc::Retained;
use objc2_app_kit::NSBitmapImageFileType;
use objc2_app_kit::NSImage;
use objc2_foundation::NSData;
use objc2_foundation::NSString;
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

    let ns_workspace = Class::get("NSWorkspace").unwrap();

    #[allow(unexpected_cfgs)]
    let workspace: *mut Object = unsafe { msg_send![ns_workspace, sharedWorkspace] };
    #[allow(unexpected_cfgs)]
    let running_app: *mut Object = unsafe { msg_send![workspace, frontmostApplication] };

    #[allow(unexpected_cfgs)]
    let cf_string_ptr: *mut CFString = unsafe { msg_send![running_app, localizedName] };

    let app_name_cf = unsafe { CFString::wrap_under_get_rule(cf_string_ptr as *mut _) };
    let app_name = app_name_cf.to_string();

    app_name
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

    let ns_workspace = Class::get("NSWorkspace").unwrap();

    #[allow(unexpected_cfgs)]
    let workspace: *mut Object = unsafe { msg_send![ns_workspace, sharedWorkspace] };
    #[allow(unexpected_cfgs)]
    let running_app: *mut Object = unsafe { msg_send![workspace, frontmostApplication] };

    #[allow(unexpected_cfgs)]
    let ns_url_ptr: *mut Object = unsafe { msg_send![running_app, bundleURL] };

    #[allow(unexpected_cfgs)]
    let ns_string_ptr: *mut NSString = unsafe { msg_send![ns_url_ptr, path] };

    let ns_string: Retained<NSString> =
        unsafe { Retained::retain_autoreleased(ns_string_ptr).unwrap() };

    let app_path = PathBuf::from(ns_string.to_string());

    app_path
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
        let ns_workspace = Class::get("NSWorkspace").unwrap();

        #[allow(unexpected_cfgs)]
        let workspace: *mut Object = unsafe { msg_send![ns_workspace, sharedWorkspace] };
        #[allow(unexpected_cfgs)]
        let running_app: *mut Object = unsafe { msg_send![workspace, frontmostApplication] };

        #[allow(unexpected_cfgs)]
        let ns_image_ptr: *mut NSImage = unsafe { msg_send![running_app, icon] };

        let ns_image: Retained<NSImage> =
            unsafe { Retained::retain_autoreleased(ns_image_ptr).unwrap() };
        let ns_image_ptr_const: *const NSImage = Retained::as_ptr(&ns_image);
        let ns_image_ptr_raw: *mut NSImage = ns_image_ptr_const as *mut NSImage;
        let ns_image_obj_ptr: *mut Object = ns_image_ptr_raw as *mut Object;

        #[allow(unexpected_cfgs)]
        let tiff_data_ptr: *mut NSData = unsafe { msg_send![ns_image_obj_ptr, TIFFRepresentation] };

        let ns_bitmap_class = Class::get("NSBitmapImageRep").unwrap();

        #[allow(unexpected_cfgs)]
        let bitmap_rep_ptr: *mut Object =
            unsafe { msg_send![ns_bitmap_class, imageRepWithData: tiff_data_ptr] };

        let png_type = NSBitmapImageFileType::PNG;

        #[allow(unexpected_cfgs)]
        let png_data_ptr: *mut NSData = unsafe {
            msg_send![bitmap_rep_ptr, representationUsingType:png_type properties:std::ptr::null::<Object>()]
        };

        let png_data: Retained<NSData> =
            unsafe { Retained::retain_autoreleased(png_data_ptr).unwrap() };
        let png_data_ptr_const: *const NSData = Retained::as_ptr(&png_data);
        let png_data_ptr_raw: *mut NSData = png_data_ptr_const as *mut NSData;
        let png_data_obj_ptr: *mut Object = png_data_ptr_raw as *mut Object;

        #[allow(unexpected_cfgs)]
        let data_len: usize = unsafe { msg_send![png_data_obj_ptr, length] };

        #[allow(unexpected_cfgs)]
        let data_bytes_ptr: *const u8 = unsafe { msg_send![png_data_obj_ptr, bytes] };

        let data_slice: &[u8] = unsafe { std::slice::from_raw_parts(data_bytes_ptr, data_len) };

        let mut file = File::create(&current_focus_app_icon_path).unwrap();
        file.write_all(data_slice).unwrap();
    }

    current_focus_app_icon_path
}
