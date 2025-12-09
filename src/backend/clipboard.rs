use rusqlite::types::Type;
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use arboard::{Clipboard, ImageData};
use once_cell::sync::Lazy;
use std::{env, thread};

use crate::backend::macos::{
    current_focus_app_icon_path, current_focus_app_name, current_focus_app_path,
};

type SourceApp = String;
type ContentType = String;
type Content = Vec<u8>;
type Timestamp = String;

const DB_PATH: &str = "clipboard.db";
static DB_CONN: Lazy<Mutex<Connection>> = Lazy::new(|| {
    let exe_path = env::current_exe().unwrap();
    let exe_parent = exe_path.parent().unwrap();
    let db_path = exe_parent.join(DB_PATH);
    let conn = Connection::open(db_path).unwrap();

    conn.execute(
        "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY,
                source_app TEXT NOT NULL,
                content_type TEXT NOT NULL,
                content BLOB NOT NULL,
                timestamp TEXT NOT NULL DEFAULT (DATETIME('NOW', 'UTC'))
    )",
        [],
    )
    .expect("Failed to create table");

    Mutex::new(conn)
});

fn db_conn() -> MutexGuard<'static, Connection> {
    DB_CONN.lock().unwrap()
}

fn save_text(content: &str) -> rusqlite::Result<()> {
    let conn = db_conn();
    let source_app = current_focus_app_name();

    conn.execute(
        "INSERT INTO history (source_app, content_type, content) VALUES (?1, ?2, ?3)",
        params![source_app, "TEXT", content],
    )?;

    Ok(())
}

fn save_image(content: &ImageData) -> rusqlite::Result<()> {
    let conn = db_conn();
    let source_app = current_focus_app_name();

    let content_bytes = content.bytes.as_ref();

    conn.execute(
        "INSERT INTO history (source_app, content_type, content) VALUES (?1, ?2, ?3)",
        params![source_app, "IMAGE", content_bytes],
    )?;

    Ok(())
}

pub fn listen() {
    let mut clipboard = Clipboard::new().unwrap();
    let mut last_text = String::new();
    let mut last_img_hash: u64 = 0;

    loop {
        if let Ok(text) = clipboard.get_text() {
            if text != last_text {
                save_text(&text).unwrap();
                last_text = text;
                last_img_hash = 0u64;
            }
        } else if let Ok(img_data) = clipboard.get_image() {
            let img_hash = img_data
                .bytes
                .iter()
                .fold(0u64, |acc, &b| acc.wrapping_add(b as u64));

            if img_hash != last_img_hash {
                save_image(&img_data).unwrap();
                last_text.clear();
                last_img_hash = img_hash;
            }
        }

        thread::sleep(Duration::from_millis(500));
    }
}

pub fn get_recent_records(
    limit: i64,
) -> rusqlite::Result<Vec<(SourceApp, ContentType, Content, Timestamp)>> {
    let conn = db_conn();

    let mut stmt = conn.prepare(
        "SELECT source_app, content_type, content, timestamp
         FROM history
         ORDER BY timestamp DESC
         LIMIT ?1",
    )?;

    let history_iter = stmt.query_map(params![limit], |row| {
        let source_app: String = row.get(0)?;
        let content_type: String = row.get(1)?;

        let content_value: ValueRef = row.get_ref(2)?;
        let content: Vec<u8> = match content_value.data_type() {
            Type::Blob => content_value.as_blob()?.to_vec(),
            Type::Text => content_value.as_str()?.as_bytes().to_vec(),
            other => {
                return Err(rusqlite::Error::InvalidColumnType(
                    2,
                    "content".to_string(),
                    other,
                ));
            }
        };

        let timestamp: String = row.get(3)?;

        Ok((source_app, content_type, content, timestamp))
    })?;

    let results: rusqlite::Result<Vec<(String, String, Vec<u8>, String)>> = history_iter.collect();

    results
}

pub fn search_text(
    term: &str,
) -> rusqlite::Result<Vec<(SourceApp, ContentType, Content, Timestamp)>> {
    let conn = db_conn();

    let pattern = format!("%{}%", term);

    let mut stmt = conn.prepare(
        "SELECT source_app, content_type, content, timestamp
         FROM history
         WHERE content LIKE ?1 LIMIT 100",
    )?;

    let history_iter = stmt.query_map(params![pattern], |row| {
        let source_app: String = row.get(0)?;
        let content_type: String = row.get(1)?;

        let content_value: ValueRef = row.get_ref(2)?;
        let content: Vec<u8> = match content_value.data_type() {
            Type::Blob => content_value.as_blob()?.to_vec(),
            Type::Text => content_value.as_str()?.as_bytes().to_vec(),
            other => {
                return Err(rusqlite::Error::InvalidColumnType(
                    2,
                    "content".to_string(),
                    other,
                ));
            }
        };

        let timestamp: String = row.get(3)?;

        Ok((source_app, content_type, content, timestamp))
    })?;

    let results = history_iter.collect();

    results
}

pub fn run_me_for_test() {
    println!("Hello");
    db_conn();
    println!("{:?}", current_focus_app_name());
    println!("{:?}", current_focus_app_path());
    println!("{:?}", current_focus_app_icon_path());
    listen();
}
