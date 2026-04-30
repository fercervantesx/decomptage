mod agent_loop;
mod attachments;
mod codeblock;
mod history;
mod provider;
mod redact;
mod rpc;
mod session;
mod tools;

use std::path::PathBuf;
use tokio::net::UnixListener;
use tracing::{info, error};

fn socket_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("decomptage")
            .join("sidecar.sock")
    } else {
        std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join("decomptage")
            .join("sidecar.sock")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "decomptage_sidecar=info".into()),
        )
        .init();

    let path = socket_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Remove stale socket
    let _ = tokio::fs::remove_file(&path).await;

    let listener = UnixListener::bind(&path)?;
    info!("sidecar listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                tokio::spawn(session::handle_connection(stream));
            }
            Err(e) => {
                error!("accept error: {}", e);
            }
        }
    }
}
