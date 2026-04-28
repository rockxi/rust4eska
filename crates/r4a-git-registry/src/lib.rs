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

pub fn list_files(repo_path: &Path, branch: &str, pattern: &str) -> Result<Vec<String>> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(repo_path)
        .args(["ls-tree", "-r", "--name-only", branch])
        .output()
        .context("git ls-tree")?;

    if !out.status.success() {
        return Ok(Vec::new());
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let files: Vec<String> = text
        .lines()
        .filter(|line| line.ends_with(pattern))
        .map(|s| s.to_string())
        .collect();
    
    Ok(files)
}

pub fn read_file(repo_path: &Path, branch: &str, file_path: &str) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(repo_path)
        .args(["show", &format!("{}:{}", branch, file_path)])
        .output()
        .context("git show")?;

    anyhow::ensure!(out.status.success(), "git show failed for {}", file_path);
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
