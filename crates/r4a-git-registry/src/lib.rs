use anyhow::{Context, Result};
use r4a_store::Store;
use std::path::{Path, PathBuf};

pub mod handler;
pub mod registry;

/// Инициализировать bare-репозиторий если его ещё нет
pub fn init_repo(path: &Path) -> Result<()> {
    if path.join("HEAD").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;

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

pub fn default_registry_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-server").join("registry")
}

#[derive(Clone)]
pub struct RegistryState {
    pub root: PathBuf,
    pub store: Store,
}

impl RegistryState {
    pub fn new(root: PathBuf, store: Store) -> Self {
        Self { root, store }
    }
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

pub fn write_and_commit_file(
    repo_path: &Path,
    branch: &str,
    file_path: &str,
    content: &str,
    commit_message: &str,
) -> Result<()> {
    let tmp_dir = tempfile::tempdir()?;
    let tmp_path = tmp_dir.path();

    let status = std::process::Command::new("git")
        .arg("clone")
        .arg(repo_path)
        .arg(tmp_path)
        .status()
        .context("git clone")?;

    anyhow::ensure!(status.success(), "git clone failed");

    let _ = std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["checkout", "-b", branch])
        .status();

    let _ = std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["checkout", branch])
        .status();

    let file_full_path = tmp_path.join(file_path);
    if let Some(parent) = file_full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&file_full_path, content)?;

    std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["config", "user.name", "r4a-web"])
        .status()?;
    std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["config", "user.email", "r4a-web@master.local"])
        .status()?;

    std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["add", file_path])
        .status()?;
    let commit_status = std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["commit", "-m", commit_message])
        .status()?;

    if commit_status.success() {
        let push_status = std::process::Command::new("git")
            .current_dir(tmp_path)
            .args(["push", "origin", branch])
            .status()?;
        anyhow::ensure!(push_status.success(), "git push failed");
    }

    Ok(())
}

pub fn get_history(
    repo_path: &Path,
    branch: &str,
    file_path: Option<&str>,
) -> Result<Vec<r4a_core::models::CommitInfo>> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["-C"]);
    cmd.arg(repo_path);
    cmd.args(["log", branch, "--pretty=format:%H|%an|%ad|%s", "--date=iso"]);

    if let Some(f) = file_path {
        cmd.arg("--");
        cmd.arg(f);
    }

    let out = cmd.output()?;
    if !out.status.success() {
        return Ok(vec![]);
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut history = vec![];
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() == 4 {
            history.push(r4a_core::models::CommitInfo {
                hash: parts[0].to_string(),
                author: parts[1].to_string(),
                date: parts[2].to_string(),
                message: parts[3].to_string(),
            });
        }
    }
    Ok(history)
}

pub fn rollback_file(
    repo_path: &Path,
    branch: &str,
    file_path: &str,
    commit_hash: &str,
) -> Result<()> {
    let tmp_dir = tempfile::tempdir()?;
    let tmp_path = tmp_dir.path();

    std::process::Command::new("git")
        .args(["clone", "--branch", branch])
        .arg(repo_path)
        .arg(tmp_path)
        .status()?;

    let checkout_status = std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["checkout", commit_hash, "--", file_path])
        .status()?;

    anyhow::ensure!(checkout_status.success(), "git checkout failed");

    std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["config", "user.name", "r4a-web"])
        .status()?;
    std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["config", "user.email", "r4a-web@master.local"])
        .status()?;

    let commit_message = format!("Rollback {} to {}", file_path, commit_hash);
    std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["commit", "-m", &commit_message])
        .status()?;

    let push_status = std::process::Command::new("git")
        .current_dir(tmp_path)
        .args(["push", "origin", branch])
        .status()?;
    anyhow::ensure!(push_status.success(), "git push failed");

    Ok(())
}
