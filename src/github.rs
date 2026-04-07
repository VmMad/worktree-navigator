use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::types::PullRequest;

pub fn list_open_prs(repo_root: &Path) -> Result<Vec<PullRequest>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            "number,title,headRefName",
            "--limit",
            "30",
        ])
        .current_dir(repo_root)
        .output()
        .context("Failed to run gh CLI. Is it installed? (brew install gh)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    let prs: Vec<PullRequest> = serde_json::from_slice(&output.stdout)
        .context("Failed to parse gh pr list output")?;

    Ok(prs)
}
