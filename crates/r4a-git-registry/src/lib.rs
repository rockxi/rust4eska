use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub mod handler;

/// Инициализировать bare-репозиторий если его ещё нет
pub fn init_repo(path: &Path) -> Result<()> {
    if path.join("HEAD").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path)
        .with_context(|| format!("create {}", path.display()))?;

    let status = std::process::Command::new("git")
        .args(["init", "--bare", "--initial-branch=main"])
        .arg(path)
        .status()
        .context("git init --bare")?;

    anyhow::ensure!(status.success(), "git init --bare failed");

    // Разрешить push в непустой репозиторий
    let _ = std::process::Command::new("git")
        .args(["-C"])
        .arg(path)
        .args(["config", "http.receivepack", "true"])
        .status();

    tracing::info!("Initialized git repo at {}", path.display());
    Ok(())
}

/// Путь к хранилищу по умолчанию
pub fn default_git_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-server").join("git")
}
