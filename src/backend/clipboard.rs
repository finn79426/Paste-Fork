use arboard::{Clipboard, ImageData};
use base64::engine::general_purpose;
use base64::prelude::*;
use chrono::{NaiveDateTime, TimeZone, Utc};
use clipboard_master::{CallbackResult, ClipboardHandler, Master};
use image::{ImageBuffer, Rgba};
use once_cell::sync::Lazy;
use rusqlite::types::{Type, ValueRef};
use rusqlite::{params, Connection, Row};
use std::env::current_exe;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::backend::macos::{current_focus_app_icon_path, current_focus_app_name};

pub static IS_INTERNAL_PASTE: AtomicBool = AtomicBool::new(false);

const DB_PATH: &str = "clipboard.db";
static DB_CONN: Lazy<Mutex<Connection>> = Lazy::new(|| {
    let exe_path = current_exe().unwrap();
    let exe_parent = exe_path.parent().unwrap();
    let db_path = exe_parent.join(DB_PATH);
    let conn = Connection::open(db_path).unwrap();

    conn.execute(
        "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY,
                source_app TEXT NOT NULL,
                icon_path TEXT NOT NULL,
                content_type TEXT NOT NULL,
                content BLOB NOT NULL,
                timestamp TEXT NOT NULL DEFAULT (DATETIME('NOW', 'UTC'))
    )",
        [],
    )
    .expect("Failed to create table");

    Mutex::new(conn)
});

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: i64,
    pub source_app: String,
    pub icon_path: String,
    pub content_type: ContentTypes,
    pub content: String,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContentTypes {
    TEXT,
    IMAGE,
}

struct Handler {
    clipboard_ctx: Option<Clipboard>,
}

impl Handler {
    fn new() -> Self {
        Handler {
            clipboard_ctx: None,
        }
    }

    fn get_clipboard(&mut self) -> Option<&mut Clipboard> {
        if self.clipboard_ctx.is_none() {
            match Clipboard::new() {
                Ok(ctx) => self.clipboard_ctx = Some(ctx),
                Err(err) => {
                    println!("Failed to get clipboard: {}", err);
                    return None;
                }
            }
        }

        self.clipboard_ctx.as_mut()
    }
}

impl ClipboardHandler for Handler {
    /// Triggered when the system clipboard content changes.
    ///
    /// # Processing Logic
    /// 1. Loop Prevention
    ///     Checks if the change is an internal paste action (`IS_INTERNAL_PASTE`).
    ///     If so, do not save anything to the database.
    /// 2. Sensitive Data Filtering
    ///     Checks if the currently focused application is a password manager.
    ///     If so, do not save anything to the database.
    /// 3. Persistence
    ///     Save the clipboard contents to the SQLite database.
    fn on_clipboard_change(&mut self) -> CallbackResult {
        // If the "system clipboard has changed" event is triggered by our own action, do not save anything to the database.
        // Because the event is triggered due to user selected a clipboard item in our Dioxus App.
        if IS_INTERNAL_PASTE.swap(false, Ordering::Relaxed) {
            return CallbackResult::Next;
        }

        // If the "system clipboard has changed" event is triggered from a password manager app
        // DO NOT save anything to the database
        const IGNORED_APPS: &[&str] = &["Passwords", "Keychain Access", "Bitwarden"];
        let current_focus_app = current_focus_app_name();

        if IGNORED_APPS
            .iter()
            .any(|ignored_app| current_focus_app.contains(ignored_app))
        {
            return CallbackResult::Next;
        }

        // Save the clipboard contents to the SQLite database
        if let Some(clipboard) = self.get_clipboard() {
            if let Ok(text) = clipboard.get_text() {
                save_text(&text).unwrap();
            } else if let Ok(image) = clipboard.get_image() {
                save_image(&image).unwrap();
            }
        }

        CallbackResult::Next
    }
}

/// Listen to system clipboard changes.
/// When clipboard changes, save the latest item to the SQLite database
///
/// Example:
/// ```
/// use crate::backend::clipboard;
///
/// clipboard::listen(); // Start listening
/// ```
pub fn listen() {
    let handler = Handler::new();
    Master::new(handler).unwrap().run().unwrap();
}

/// Get all of the records from the SQLite database
///
/// # Example:
/// ```
/// use crate::backend::clipboard;
///
/// let records = clipboard::get_all_records();
/// println!("{:?}", records); // Output: Ok([Item { id: 1, source_app: "Code", icon_path: "/foo/bar/Code.png", content_type: TEXT, content: "Hello", timestamp: 2025-12-27T17:11:28Z }])
pub fn get_all_records() -> rusqlite::Result<Vec<Item>> {
    let conn = db_conn();

    let mut stmt = conn.prepare(
        "SELECT id, source_app, icon_path, content_type, content, timestamp
         FROM history
         ORDER BY timestamp DESC",
    )?;

    let history_iter = stmt.query_map(params![], row_to_item)?;

    history_iter.collect()
}

/// Get the latest records from the SQLite database
///
/// # Arguments
///
/// * `limit` - The number of records to return
///
/// # Example:
/// ```
/// use crate::backend::clipboard;
///
/// let records = clipboard::get_recent_records(1);
/// println!("{:?}", records); // Output: Ok([Item { id: 1, source_app: "Code", icon_path: "/foo/bar/Code.png", content_type: TEXT, content: "Hello", timestamp: 2025-12-27T17:11:28Z }])
/// ```
pub fn get_recent_records(limit: i64) -> rusqlite::Result<Vec<Item>> {
    let conn = db_conn();

    let mut stmt = conn.prepare(
        "SELECT id, source_app, icon_path, content_type, content, timestamp
         FROM history
         ORDER BY timestamp DESC
         LIMIT ?1",
    )?;

    let history_iter = stmt.query_map(params![limit], row_to_item)?;

    history_iter.collect()
}

/// Search for specific text in the SQLite database
///
/// # Arguments
///
/// * `term` - The text to search for
///
/// # Example:
/// ```
/// use crate::backend::clipboard;
///
/// let records = clipboard::search_text("Hello World");
/// println!("{:?}", records); // Output: Ok([Item { id: 1, source_app: "Code", icon_path: "/foo/bar/Code.png", content_type: TEXT, content: "Hello World", timestamp: 2025-12-27T17:28:01Z }])
/// ```
pub fn search_text(term: &str) -> rusqlite::Result<Vec<Item>> {
    let conn = db_conn();
    let pattern = format!("%{}%", term);

    let mut stmt = conn.prepare(
        "SELECT id, source_app, icon_path, content_type, content, timestamp
         FROM history
         WHERE content_type = 'TEXT' AND content LIKE ?1
         ORDER BY timestamp DESC
        ",
    )?;

    let history_iter = stmt.query_map(params![pattern], row_to_item)?;

    history_iter.collect()
}

/// Updates the timestamp of a specific history record.
///
/// This function sets the `timestamp` column of the item with the given `id`
/// to the **current UTC time** in the `history` database table.
///
/// # Purpose
/// This is typically used when a user re-pastes an old clipboard item.
/// By updating the timestamp, the item is effectively "bumped" to the
/// **top of the list** (marked as most recently used).
///
/// # Arguments
///
/// * `id` - The unique identifier (Primary Key) of the history record.
///
/// # Example
/// ```
/// use crate::backend::clipboard;
///
/// clipboard::update_timestamp(1);
/// ```
pub fn update_timestamp(id: i64) -> rusqlite::Result<()> {
    let conn = db_conn();

    conn.execute(
        "UPDATE history SET timestamp = DATETIME('NOW', 'UTC') WHERE id = ?1",
        params![id],
    )?;

    Ok(())
}

/// Saves text content to the clipboard history database.
///
/// It automatically captures context metadata:
/// - `source_app`: Name of the focused application.
/// - `icon_path`: Path to the application's icon.
///
/// # Arguments
///
/// * `content` - The text string to be saved.
fn save_text(content: &str) -> rusqlite::Result<()> {
    let conn = db_conn();
    let source_app = current_focus_app_name();
    let icon_path = current_focus_app_icon_path().to_string_lossy().to_string();

    let row_effected = conn.execute(
        "
        UPDATE history
        SET timestamp = DATETIME('NOW', 'UTC'), source_app = ?1, icon_path = ?2
        WHERE content_type = 'TEXT' AND content = ?3
    ",
        params![source_app, icon_path, content],
    )?;

    if row_effected == 0 {
        conn.execute(
            "INSERT INTO history (source_app, icon_path, content_type, content) VALUES (?1, ?2, 'TEXT', ?3)",
            params![source_app, icon_path, content],
        )?;
    }

    Ok(())
}

/// Saves image content to the clipboard history database.
///
/// Similar to `save_text` function.
///
/// # Arguments
///
/// * `content` - The raw image data captured from the system clipboard.
fn save_image(content: &ImageData) -> rusqlite::Result<()> {
    let conn = db_conn();
    let source_app = current_focus_app_name();
    let icon_path = current_focus_app_icon_path().to_string_lossy().to_string();
    let content_bytes = content.bytes.as_ref();
    let width = content.width as u32;
    let height = content.height as u32;
    let png_bytes = if let Some(img_buffer) =
        ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, content_bytes)
    {
        let mut bytes: Vec<u8> = Vec::new();
        if let Ok(_) = img_buffer.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png) {
            bytes
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let rows_affected = conn.execute(
        "UPDATE history 
         SET timestamp = DATETIME('NOW', 'UTC'), source_app = ?1, icon_path = ?2
         WHERE content_type = 'IMAGE' AND content = ?3",
        params![source_app, icon_path, png_bytes],
    )?;

    if rows_affected == 0 {
        conn.execute(
            "INSERT INTO history (source_app, icon_path, content_type, content) VALUES (?1, ?2, 'IMAGE', ?3)",
            params![source_app, icon_path, png_bytes],
        )?;
    }

    Ok(())
}

/// Maps a raw database row to the `Item` struct.
///
/// # Arguments
///
/// * `row` - A reference to a single row result from a rusqlite query.
fn row_to_item(row: &Row) -> rusqlite::Result<Item> {
    let id: i64 = row.get(0)?;
    let source_app: String = row.get(1)?;
    let icon_path: String = row.get(2)?;
    let content_type: String = row.get(3)?;
    let content: ValueRef = row.get_ref(4)?;
    let timestamp: String = row.get(5)?;

    let content_type = match content_type.as_str() {
        "IMAGE" => ContentTypes::IMAGE,
        "TEXT" => ContentTypes::TEXT,
        _ => unreachable!(),
    };

    let content_raw_bytes: Vec<u8> = match content.data_type() {
        Type::Blob => content.as_blob()?.to_vec(),
        Type::Text => content.as_str()?.as_bytes().to_vec(),
        _ => Vec::new(),
    };

    let content = match content_type {
        ContentTypes::IMAGE => general_purpose::STANDARD.encode(&content_raw_bytes),
        ContentTypes::TEXT => String::from_utf8_lossy(&content_raw_bytes).to_string(),
    };

    let timestamp = NaiveDateTime::parse_from_str(&timestamp, "%Y-%m-%d %H:%M:%S")
        .map(|naive| Utc.from_utc_datetime(&naive))
        .unwrap_or_else(|_| Utc::now());

    Ok(Item {
        id,
        source_app,
        icon_path,
        content_type,
        content,
        timestamp,
    })
}

/// Get a connection to the SQLite database
fn db_conn() -> MutexGuard<'static, Connection> {
    DB_CONN.lock().unwrap()
}
