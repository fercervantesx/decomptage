use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

/// Maximum attachment size: 20MB
const MAX_ATTACHMENT_SIZE: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub path: PathBuf,
    pub size: u64,
}

fn cache_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("decomptage/attachments")
    } else {
        PathBuf::from("/tmp/decomptage/attachments")
    }
}

pub async fn store_attachment(
    name: &str,
    mime_type: &str,
    data: &[u8],
) -> Result<Attachment, String> {
    if data.len() as u64 > MAX_ATTACHMENT_SIZE {
        return Err(format!(
            "attachment too large: {} bytes (max {})",
            data.len(),
            MAX_ATTACHMENT_SIZE
        ));
    }

    let dir = cache_dir();
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("failed to create cache dir: {}", e))?;

    let ext = name.rsplit('.').next().unwrap_or("bin");
    let id = Uuid::new_v4().to_string();
    let filename = format!("{}.{}", id, ext);
    let path = dir.join(&filename);

    fs::write(&path, data)
        .await
        .map_err(|e| format!("failed to write attachment: {}", e))?;

    Ok(Attachment {
        id,
        name: name.to_string(),
        mime_type: mime_type.to_string(),
        path,
        size: data.len() as u64,
    })
}

pub async fn load_attachment(id: &str) -> Result<(String, Vec<u8>), String> {
    let dir = cache_dir();
    let mut entries = fs::read_dir(&dir)
        .await
        .map_err(|e| format!("failed to read cache dir: {}", e))?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(id) {
            let data = fs::read(entry.path())
                .await
                .map_err(|e| format!("failed to read: {}", e))?;
            let mime = mime_from_extension(&name);
            return Ok((mime, data));
        }
    }

    Err(format!("attachment not found: {}", id))
}

/// Convert attachment to base64 for providers that accept inline images.
pub fn to_base64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn mime_from_extension(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "json" => "application/json",
        "md" => "text/markdown",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Cleanup old attachments (older than 30 days)
pub async fn gc_old_attachments() {
    let dir = cache_dir();
    if !dir.exists() {
        return;
    }

    let thirty_days = std::time::Duration::from_secs(30 * 24 * 60 * 60);
    let cutoff = std::time::SystemTime::now() - thirty_days;

    if let Ok(mut entries) = fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = fs::remove_file(entry.path()).await;
                    }
                }
            }
        }
    }
}
