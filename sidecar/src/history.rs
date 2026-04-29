use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

/// Simple file-based chat history persistence.
/// Each chat is stored as a JSON file in the history directory.
/// Phase 7 MVP — will migrate to SQLite later for query performance.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistory {
    pub chat_id: String,
    pub provider: String,
    pub model: String,
    pub messages: Vec<HistoryMessage>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub timestamp: u64,
}

fn history_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("decomptage/history")
    } else {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("decomptage/history")
    }
}

pub async fn save_chat(history: &ChatHistory) -> Result<(), String> {
    let dir = history_dir();
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("mkdir failed: {}", e))?;

    let path = dir.join(format!("{}.json", history.chat_id));
    let json = serde_json::to_string_pretty(history)
        .map_err(|e| format!("serialize failed: {}", e))?;

    fs::write(&path, json)
        .await
        .map_err(|e| format!("write failed: {}", e))?;

    Ok(())
}

pub async fn load_chat(chat_id: &str) -> Result<ChatHistory, String> {
    let path = history_dir().join(format!("{}.json", chat_id));
    let data = fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read failed: {}", e))?;
    serde_json::from_str(&data).map_err(|e| format!("parse failed: {}", e))
}

pub async fn list_chats() -> Result<Vec<ChatHistory>, String> {
    let dir = history_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut result = Vec::new();
    let mut entries = fs::read_dir(&dir)
        .await
        .map_err(|e| format!("readdir failed: {}", e))?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(data) = fs::read_to_string(&path).await {
                if let Ok(history) = serde_json::from_str::<ChatHistory>(&data) {
                    result.push(history);
                }
            }
        }
    }

    result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(result)
}

pub async fn delete_chat(chat_id: &str) -> Result<(), String> {
    let path = history_dir().join(format!("{}.json", chat_id));
    fs::remove_file(&path)
        .await
        .map_err(|e| format!("delete failed: {}", e))?;
    Ok(())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
