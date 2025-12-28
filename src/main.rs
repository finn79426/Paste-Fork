mod backend;

use arboard::Clipboard;
use dioxus::html::{input_data::keyboard_types::Key};
use dioxus::prelude::*;
use dioxus_desktop::{
    tao::dpi::{LogicalPosition, LogicalSize},
    use_global_shortcut, use_window, Config, WindowBuilder,
};
use global_hotkey::HotKeyState;
use once_cell::sync::Lazy;
use std::process::Command;
use std::sync::{Arc, RwLock};
use std::thread;
use std::{collections::HashMap, sync::atomic::Ordering};
use tokio::sync::mpsc;

use crate::backend::clipboard::{self, ContentTypes};
use crate::backend::clipboard::{update_timestamp, IS_INTERNAL_PASTE};
use crate::backend::utils::{b64_to_img_data, humanize_time};

const MAIN_CSS: Asset = asset!("/assets/main.css");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[derive(Clone)]
pub struct WindowInfo {
    pub is_visible: bool, // represents the current window's status is visible or not
    pub visibility_setter: mpsc::UnboundedSender<bool>, // A mpsc sender for setting `is_visible`
}

static WINDOW_REGISTRY: Lazy<Arc<RwLock<HashMap<String, WindowInfo>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

// ------------------------------------------------------------------
//                            MAIN ENTRY
// ------------------------------------------------------------------
fn main() {
    let config = Config::new().with_window(default_app_window_config());

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

// ------------------------------------------------------------------
//                              CONFIG
// ------------------------------------------------------------------
fn default_app_window_config() -> WindowBuilder {
    WindowBuilder::new()
        .with_always_on_top(true)
        .with_content_protection(true)
        .with_decorations(false)
        .with_focused(false)
        .with_resizable(false)
        .with_title("App")
        .with_transparent(true)
        .with_visible(false)
}

fn default_paste_window_config() -> WindowBuilder {
    WindowBuilder::new()
        .with_always_on_top(true)
        .with_content_protection(true)
        .with_decorations(false)
        .with_focused(true)
        .with_resizable(false)
        .with_title("Paste")
        .with_transparent(true)
        .with_visible(false)
}

// ------------------------------------------------------------------
//                            COMPONENTS
// ------------------------------------------------------------------
#[component]
fn App() -> Element {
    let window = use_window();

    // Initialize child windows
    use_effect(move || {
        window.new_window(
            VirtualDom::new(Paste),
            Config::new().with_window(default_paste_window_config()),
        );
    });

    // Hotkey: A toggle for setting the visibility of `Paste` window
    use_global_shortcut("SHIFT+CMD+V", move |s| {
        if let HotKeyState::Pressed = s {
            if let Ok(mut registry) = WINDOW_REGISTRY.write() {
                if let Some(info) = registry.get_mut("Paste") {
                    info.visibility_setter.send(!info.is_visible).unwrap();
                }
            }
        }
    })
    .unwrap();

    rsx!("")
}

#[component]
fn Paste() -> Element {
    let window = use_window();
    let mut clipboard_items = use_signal(Vec::<clipboard::Item>::new);
    let mut search_bar = use_signal(|| "".to_string());
    let mut selected_item_index = use_signal(|| 0);

    // Change Window Size
    use_effect({
        to_owned![window];

        move || {
            to_owned![window];
            spawn(async move {
                if let Some(monitor) = window.current_monitor() {
                    let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
                    let new_height = size.height / 3.3;
                    let new_y_pos = size.height - new_height;
                    window.set_inner_size(LogicalSize::new(size.width, new_height));
                    window.set_outer_position(LogicalPosition::new(0.0, new_y_pos));
                }
            });
        }
    });

    // A hook to filter the clipboard items based on the user input
    // The search bar is used to filter the clipboard items
    let filtered_items = use_memo(move || {
        let query = search_bar.read().to_lowercase();
        let clipboard_items = clipboard_items.read();

        if query.is_empty() {
            log::trace!("Query is empty");
            clipboard_items.clone()
        } else {
            log::trace!("User input: {}", query);
            clipboard_items
                .iter()
                .filter(|item| {
                    item.source_app.to_lowercase().contains(&query)
                        || item.content.to_lowercase().contains(&query)
                })
                .cloned()
                .collect()
        }
    });

    // A hook to set the visibility of the `Paste` window
    // A unbounded channel has been used to toggle the visibility of the `Paste` window
    let visibility_setter = use_hook(|| {
        to_owned![window];
        let (tx, mut rx) = mpsc::unbounded_channel::<bool>();

        spawn(async move {
            loop {
                tokio::select! {
                    should_show = rx.recv() => {
                        if let Some(should_show) = should_show {
                            if should_show {
                                log::trace!("Showing Window");
                                window.set_visible(true);
                                window.set_focus();
                            } else {
                                log::trace!("Hiding Window");
                                window.set_visible(false);
                            }
                            set_window_visibility("Paste", should_show);
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {
                        if window.is_visible() && !window.is_focused() {
                            log::trace!("Hiding Window due to losing focus");
                            window.set_visible(false);
                            set_window_visibility("Paste", false);
                        }
                    }
                }
            }
        });
        tx
    });

    // Start listening to system clipboard after component rendered
    use_effect(move || {
        let (tx, mut rx) = mpsc::unbounded_channel::<()>();
        thread::spawn(move || clipboard::listen(tx));

        spawn(async move {
            while rx.recv().await.is_some() {
                log::trace!("Received clipboard DB completed updating signal");
                clipboard_items.set(clipboard::get_all_records().unwrap()); // BUG: Memory could goes insufficient if `get_all_records` returns massive amount of data
            }
        });
    });

    // Register the `Paste` window to the window registry after component rendered
    use_effect({
        to_owned![visibility_setter];

        move || {
            if let Ok(mut registry) = WINDOW_REGISTRY.write() {
                registry.insert(
                    "Paste".to_string(),
                    WindowInfo {
                        visibility_setter: visibility_setter.clone(),
                        is_visible: false,
                    },
                );
            }
            log::trace!("Paste window registered");
        }
    });

    // Action Handler `do_paste`: Copy the selected clipboard item
    // Triggered when user select a clipboard item
    let do_paste = {
        to_owned![visibility_setter];

        move |item: clipboard::Item| {
            spawn(async move {
                // BE Update: update system clipboard
                let mut clipboard = Clipboard::new().unwrap();

                IS_INTERNAL_PASTE.store(true, Ordering::SeqCst);

                if item.content_type == ContentTypes::Text {
                    clipboard.set_text(&item.content).unwrap();
                } else {
                    clipboard.set_image(b64_to_img_data(&item.content)).unwrap();
                }

                // DB Update: Update the selected item's timestamp to now
                update_timestamp(item.id).unwrap();

                // UI Update: Move the selected item to the index[0]
                let mut clipboard_items = clipboard_items.write();
                if let Some(pos) = clipboard_items.iter().position(|i| i.id == item.id) {
                    let item = clipboard_items.remove(pos);
                    clipboard_items.insert(0, item);
                }

                // UI Update: Reset the search bar and selected index
                search_bar.set("".to_string());
                selected_item_index.set(0);

                // UI Update: Hide the window
                visibility_setter.send(false).unwrap();

                // UX Update: refocusing preview application
                Command::new("osascript")
                    .arg("-e")
                    .arg(
                        r#"
                        tell application "System Events"
                            set visible of first process whose frontmost is true to false
                        end tell
                    "#,
                    )
                    .output()
                    .unwrap();

                // TODO UX Update: Automatically pasting
                // Currently not supported.
                // Pasting immediately after user selection would require integration with macOS system APIs.
            });
            
        }
    };

    // Keyboard handler: User can use arrow keys to navigate the clipboard items
    let handle_keydown = {
        to_owned![visibility_setter, do_paste];

        move |evt: KeyboardEvent| {
            to_owned![do_paste];

            let max_len = filtered_items.read().len();
            let filtered_items = filtered_items.read();

            if max_len == 0 {
                return;
            }

            match evt.key() {
                Key::ArrowRight => {
                    let current_idx = *selected_item_index.read();
                    selected_item_index.set((current_idx + 1) % max_len);
                }
                Key::ArrowLeft => {
                    let current_idx = *selected_item_index.read();
                    selected_item_index.set(if current_idx == 0 {
                        max_len - 1
                    } else {
                        current_idx - 1
                    });
                }
                Key::Character(c) => {
                    if evt.modifiers().contains(Modifiers::META) {
                        if let Ok(digit) = c.parse::<usize>() {
                            let idx = match digit {
                                0 => 9,
                                n => n.saturating_sub(1),
                            };

                            if let Some(item) = filtered_items.get(idx) {
                                do_paste(item.clone());
                            }
                        }
                    }
                }
                Key::Enter => {
                    if let Some(item) = filtered_items.get(*selected_item_index.read()) {
                        do_paste(item.clone());
                    }
                    visibility_setter.send(false).unwrap();
                }
                Key::Escape => {
                    visibility_setter.send(false).unwrap();
                }
                _ => {}
            }
        }
    };

    rsx! {
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        div {
            class: "fixed inset-0 w-full h-full bg-transparent flex items-center justify-center p-3",
            onkeydown: handle_keydown,

            div {
                class: "w-full h-full bg-[#252526] text-white flex flex-col overflow-hidden rounded-2xl shadow-2xl border border-white/10 relative",

                // Header (Search Bar, Item Count)
                div {
                    class: "flex-none w-full h-[60px] flex items-center px-6 bg-[#333333] shadow-md z-10 pt-1",
                    div { class: "mr-3 text-2xl", "🔍" }
                    input {
                        class: "flex-1 bg-transparent border-none outline-none text-xl text-white placeholder-gray-500 font-light",
                        placeholder: "Type to search...",
                        value: "{search_bar}",
                        oninput: move |evt| { search_bar.set(evt.value()); selected_item_index.set(0); },
                        autofocus: true,
                    }
                    div { class: "text-gray-500 text-sm font-mono", "{filtered_items.read().len()} items" }
                }

                // Body (Items)
                div {
                    class: "flex-1 w-full overflow-x-auto overflow-y-hidden flex flex-row items-center gap-5 px-6 scrollbar-hide bg-[#1e1e1e]",

                    if filtered_items.read().is_empty() {
                            div { class: "w-full text-center text-gray-500 text-xl", "No records found 🕵️‍♂️" }
                    } else {
                        {
                            filtered_items.read().iter().enumerate().map(|(index, item)| {
                                to_owned![do_paste, item];

                                rsx! {
                                    ClipboardCard {
                                        key: "{item.id}",
                                        index: index,
                                        is_selected: index == *selected_item_index.read(),
                                        item: item.clone(),
                                        on_click: move |_| {
                                            to_owned![do_paste];
                                            if index == *selected_item_index.read() {
                                                do_paste(item.clone());
                                            } else {
                                                selected_item_index.set(index);
                                            }
                                        }
                                    }
                                }
                            })
                        }
                    }
                }

                // Footer
                div {
                    class: "flex-none h-[24px] bg-[#007acc] flex items-center justify-between px-3 text-xs font-semibold text-white select-none",

                    div {
                        class: "flex items-center gap-4",

                        div { class: "flex items-center gap-1",
                            span { "Esc" }
                            span { class: "opacity-80", "Close" }
                        }

                        div { class: "flex items-center gap-1",
                            span { "Enter" }
                            span { class: "opacity-80", "Paste" }
                        }

                        div { class: "flex items-center gap-1",
                            span { "← →" }
                            span { class: "opacity-80", "Select" }
                        }
                    }

                    span {
                        class: "opacity-90",
                        "Paste for Rust"
                    }
                }
            }
        }
    }
}

#[component]
fn ClipboardCard(
    index: usize,
    is_selected: bool,
    item: clipboard::Item,
    on_click: EventHandler<()>,
) -> Element {
    let base_style = "flex-shrink-0 w-[240px] h-[180px] rounded-lg flex flex-col cursor-pointer relative overflow-hidden transition-all duration-200";
    let active_style = if is_selected {
        "ring-4 ring-blue-500 bg-[#3c3c3c] scale-105 shadow-2xl z-10"
    } else {
        "bg-[#2d2d2d] hover:bg-[#333333] opacity-80 hover:opacity-100"
    };

    rsx! {
        div {
            class: "{base_style} {active_style}",
            onclick: move |_| on_click.call(()),

            // Header: SourceApp, RelativeTimestamp, Icon
            div {
                class: "h-12 px-3 flex items-center justify-between bg-black/20 border-b border-white/5",

                // Left: SourceApp, RelativeTimestamp
                div {
                    class: "flex flex-col justify-center",
                    span { class: "text-sm font-bold text-gray-200 truncate max-w-[180px]", "{item.source_app}" }
                    span { class: "text-[10px] text-gray-500 font-mono mt-0.5", "{humanize_time(item.timestamp)}" }
                }

                // Right: App Icon
                div {
                    class: "w-8 h-8 rounded bg-white/10 p-1 flex items-center justify-center shadow-inner",
                    img {
                        class: "w-full h-full object-contain",
                        alt: "App Icon",
                        src: "{item.icon_path}"
                    }
                }
            }

            // Content
            div {
                class: "flex-1 p-3 overflow-hidden text-xs text-gray-300 font-mono leading-relaxed break-all whitespace-pre-wrap [mask-image:linear-gradient(to_bottom,black_70%,transparent)]",
                if item.content_type == ContentTypes::Text {
                    "{&item.content}"
                } else if item.content_type == ContentTypes::Image {
                    img {
                        class: "w-full h-full object-contain block",
                        alt: "Image Preview",
                        src: "data:image/png;base64,{&item.content}"
                    }
                }
            }

            // Shortcut Hint
            if index < 9 {
                div { class: "absolute bottom-2 right-2 px-2 py-0.5 rounded bg-black/50 text-xs text-gray-500 font-bold", "⌘{index + 1}" }
            }
        }
    }
}

// ------------------------------------------------------------------
//                             INTERNAL
// ------------------------------------------------------------------
/// A helper function to set the visibility of a window
fn set_window_visibility(name: &str, is_visible: bool) {
    if let Ok(mut registry) = WINDOW_REGISTRY.write() {
        if let Some(info) = registry.get_mut(name) {
            info.is_visible = is_visible;
        }
    }
}
