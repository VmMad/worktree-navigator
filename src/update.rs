use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const REPO_FULL_NAME: &str = "VmMad/worktree-navigator";
const CACHE_TTL_SECS: u64 = 60 * 60 * 6;
const UPDATE_CACHE_FILE: &str = "update-check.json";
const AUTO_UPDATE_SUPPRESS_FILE: &str = "auto-update-suppressed";
const INSTALL_STATE_FILE: &str = "install-state.json";
const BINARY_NAME_PREFIX: &str = "worktree-navigator-";

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCache {
    last_checked_unix: u64,
    latest_version: String,
    #[serde(default)]
    latest_asset_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct InstallState {
    #[serde(default)]
    preferred_asset_name: Option<String>,
}

#[derive(Debug)]
struct LatestRelease {
    version: String,
    asset_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
}

pub enum StartupUpdateAction {
    Continue,
    ExitAfterUpdateFlow,
}

pub fn maybe_prompt_for_update() -> Result<StartupUpdateAction> {
    if is_auto_update_suppressed() {
        return Ok(StartupUpdateAction::Continue);
    }

    let current = normalize_version(env!("CARGO_PKG_VERSION"));
    let Some(latest) = latest_release_quick()? else {
        return Ok(StartupUpdateAction::Continue);
    };

    if !is_newer_version(&latest.version, &current) {
        return Ok(StartupUpdateAction::Continue);
    }

    // Skip prompt when we cannot map this installation to a compatible release asset.
    if latest.asset_name.is_none() {
        return Ok(StartupUpdateAction::Continue);
    }

    if !io::stdin().is_terminal() {
        return Ok(StartupUpdateAction::Continue);
    }

    let mut stderr = io::stderr();
    writeln!(
        stderr,
        "Update available for wt: v{} (current: v{})",
        latest.version, current
    )?;
    write!(stderr, "Update now? [y/N]: ")?;
    stderr.flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let accepted = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");

    if accepted {
        run_update_internal(latest.asset_name.as_deref(), Some(latest.version.as_str()))?;
        writeln!(
            stderr,
            "wt updated. Run `wt` again to start the new version."
        )?;
        Ok(StartupUpdateAction::ExitAfterUpdateFlow)
    } else {
        suppress_auto_updates()?;
        writeln!(
            stderr,
            "Auto-update prompts disabled. Use `wt --update` whenever you want to update."
        )?;
        Ok(StartupUpdateAction::Continue)
    }
}

pub fn run_manual_update() -> Result<()> {
    let latest = latest_release_quick()?;
    run_update_internal(
        latest.as_ref().and_then(|r| r.asset_name.as_deref()),
        latest.as_ref().map(|r| r.version.as_str()),
    )
}

fn run_update_internal(asset_name_hint: Option<&str>, latest_hint: Option<&str>) -> Result<()> {
    let asset_name = asset_name_hint.ok_or_else(|| {
        anyhow::anyhow!(
            "No compatible release asset found for this wt binary. Use manual install for your platform."
        )
    })?;
    let target = update_target_binary()?;
    let parent = target
        .parent()
        .map(PathBuf::from)
        .context("Could not determine target binary directory")?;
    fs::create_dir_all(&parent).context("Failed to create target directory for wt binary")?;

    let tmp = parent.join(format!(
        ".wt-update-{}-{}",
        std::process::id(),
        now_unix_seconds()
    ));

    let tmp_str = tmp.to_string_lossy().to_string();
    let download = Command::new("gh")
        .args([
            "release",
            "download",
            "--repo",
            REPO_FULL_NAME,
            "--pattern",
            asset_name,
            "--output",
            &tmp_str,
            "--clobber",
        ])
        .output()
        .context("Failed to run `gh release download` while updating wt")?;

    if !download.status.success() {
        let stderr = String::from_utf8_lossy(&download.stderr);
        anyhow::bail!("Update failed: {}", stderr.trim());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
            .context("Failed to make updated wt binary executable")?;
    }

    fs::rename(&tmp, &target).context("Failed to replace wt binary with updated version")?;
    clear_auto_update_suppression();
    let _ = write_install_state(&InstallState {
        preferred_asset_name: Some(asset_name.to_string()),
    });

    let mut stderr = io::stderr();
    if let Some(latest) = latest_hint {
        writeln!(
            stderr,
            "Updated wt to v{} at {}",
            latest,
            target.to_string_lossy()
        )?;
    } else {
        writeln!(stderr, "Updated wt at {}", target.to_string_lossy())?;
    }

    Ok(())
}

fn latest_release_quick() -> Result<Option<LatestRelease>> {
    let install_state = read_install_state();
    let preferred_asset = install_state
        .as_ref()
        .and_then(|s| s.preferred_asset_name.as_deref());
    let cache = read_update_cache();
    let now = now_unix_seconds();
    if let Some(cache) = &cache {
        let age = now.saturating_sub(cache.last_checked_unix);
        if age <= CACHE_TTL_SECS && !cache.latest_version.is_empty() {
            let asset_name = cache
                .latest_asset_name
                .clone()
                .or_else(|| preferred_asset.map(str::to_string));
            return Ok(Some(LatestRelease {
                version: cache.latest_version.clone(),
                asset_name,
            }));
        }
    }

    // Keep startup snappy by bounding this call to ~1 second.
    let out = Command::new("timeout")
        .args([
            "1",
            "gh",
            "api",
            &format!("repos/{REPO_FULL_NAME}/releases/latest"),
        ])
        .output();

    let Ok(out) = out else {
        return Ok(cache.map(|c| LatestRelease {
            version: c.latest_version,
            asset_name: c
                .latest_asset_name
                .or_else(|| preferred_asset.map(str::to_string)),
        }));
    };
    if !out.status.success() {
        return Ok(cache.map(|c| LatestRelease {
            version: c.latest_version,
            asset_name: c
                .latest_asset_name
                .or_else(|| preferred_asset.map(str::to_string)),
        }));
    }

    let release: GhRelease = match serde_json::from_slice(&out.stdout) {
        Ok(r) => r,
        Err(_) => {
            return Ok(cache.map(|c| LatestRelease {
                version: c.latest_version,
                asset_name: c
                    .latest_asset_name
                    .or_else(|| preferred_asset.map(str::to_string)),
            }));
        }
    };

    if release.tag_name.trim().is_empty() {
        return Ok(cache.map(|c| LatestRelease {
            version: c.latest_version,
            asset_name: c
                .latest_asset_name
                .or_else(|| preferred_asset.map(str::to_string)),
        }));
    }

    let latest = normalize_version(&release.tag_name);
    let asset_names: Vec<String> = release.assets.into_iter().map(|a| a.name).collect();
    let selected_asset = select_asset_name(&asset_names, preferred_asset);

    let cache_value = UpdateCache {
        last_checked_unix: now,
        latest_version: latest.clone(),
        latest_asset_name: selected_asset.clone(),
    };
    let _ = write_update_cache(&cache_value);
    if selected_asset.is_some() {
        let _ = write_install_state(&InstallState {
            preferred_asset_name: selected_asset.clone(),
        });
    }

    Ok(Some(LatestRelease {
        version: latest,
        asset_name: selected_asset,
    }))
}

fn normalize_version(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string()
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    let a = parse_version_numbers(candidate);
    let b = parse_version_numbers(current);
    let max_len = a.len().max(b.len());

    for i in 0..max_len {
        let av = *a.get(i).unwrap_or(&0);
        let bv = *b.get(i).unwrap_or(&0);
        if av > bv {
            return true;
        }
        if av < bv {
            return false;
        }
    }
    false
}

fn parse_version_numbers(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .filter_map(|digits| digits.parse::<u64>().ok())
        .collect()
}

fn select_asset_name(assets: &[String], preferred: Option<&str>) -> Option<String> {
    if let Ok(explicit) = std::env::var("WT_UPDATE_ASSET") {
        if let Some(found) = assets.iter().find(|a| a.as_str() == explicit.as_str()) {
            return Some(found.clone());
        }
    }

    if let Some(pref) = preferred {
        if let Some(found) = assets.iter().find(|a| a.as_str() == pref) {
            return Some(found.clone());
        }
    }

    let arch_tokens = current_arch_tokens();
    let os_tokens = current_os_tokens();
    let env_tokens = current_env_tokens();

    let mut best: Option<(usize, String)> = None;

    for asset in assets {
        let lower = asset.to_ascii_lowercase();
        if !lower.starts_with(BINARY_NAME_PREFIX) {
            continue;
        }
        if !arch_tokens.iter().any(|t| lower.contains(t)) {
            continue;
        }
        if !os_tokens.iter().any(|t| lower.contains(t)) {
            continue;
        }
        if !env_tokens.is_empty() && !env_tokens.iter().any(|t| lower.contains(t)) {
            continue;
        }

        let mut score = 0usize;
        if lower.contains(&format!("{}{}", BINARY_NAME_PREFIX, arch_tokens[0])) {
            score += 3;
        }
        if let Some(primary_env) = env_tokens.first() {
            if lower.contains(primary_env) {
                score += 1;
            }
        }

        match &best {
            Some((best_score, _)) if *best_score >= score => {}
            _ => best = Some((score, asset.clone())),
        }
    }

    best.map(|(_, name)| name)
}

fn current_arch_tokens() -> Vec<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => vec!["x86_64", "amd64"],
        "aarch64" => vec!["aarch64", "arm64"],
        "x86" => vec!["i686", "x86"],
        "arm" => vec!["armv7", "arm"],
        other => vec![other],
    }
}

fn current_os_tokens() -> Vec<&'static str> {
    match std::env::consts::OS {
        "linux" => vec!["linux"],
        "macos" => vec!["apple-darwin", "darwin", "macos"],
        "windows" => vec!["pc-windows-msvc", "pc-windows-gnu", "windows"],
        other => vec![other],
    }
}

fn current_env_tokens() -> Vec<&'static str> {
    let mut envs = Vec::new();
    if cfg!(target_env = "gnu") {
        envs.push("gnu");
    }
    if cfg!(target_env = "musl") {
        envs.push("musl");
    }
    if cfg!(target_env = "msvc") {
        envs.push("msvc");
    }
    envs
}

fn update_target_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("WT_UPDATE_TARGET") {
        return Ok(PathBuf::from(path));
    }

    let current = std::env::current_exe().context("Failed to resolve current wt binary path")?;
    if current
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n == "wt")
        .unwrap_or(false)
    {
        return Ok(current);
    }

    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/bin/wt"))
}

fn read_update_cache() -> Option<UpdateCache> {
    let path = update_state_dir().join(UPDATE_CACHE_FILE);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_update_cache(cache: &UpdateCache) -> Result<()> {
    let dir = update_state_dir();
    fs::create_dir_all(&dir).context("Failed to create update state directory")?;
    let data = serde_json::to_string(cache).context("Failed to serialize update cache")?;
    fs::write(dir.join(UPDATE_CACHE_FILE), data).context("Failed to write update cache")?;
    Ok(())
}

fn read_install_state() -> Option<InstallState> {
    let path = update_state_dir().join(INSTALL_STATE_FILE);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_install_state(state: &InstallState) -> Result<()> {
    let dir = update_state_dir();
    fs::create_dir_all(&dir).context("Failed to create update state directory")?;
    let data = serde_json::to_string(state).context("Failed to serialize install state")?;
    fs::write(dir.join(INSTALL_STATE_FILE), data).context("Failed to write install state")?;
    Ok(())
}

fn is_auto_update_suppressed() -> bool {
    update_state_dir().join(AUTO_UPDATE_SUPPRESS_FILE).exists()
}

fn suppress_auto_updates() -> Result<()> {
    let dir = update_state_dir();
    fs::create_dir_all(&dir).context("Failed to create update state directory")?;
    fs::write(dir.join(AUTO_UPDATE_SUPPRESS_FILE), "")
        .context("Failed to persist auto-update preference")?;
    Ok(())
}

fn clear_auto_update_suppression() {
    let _ = fs::remove_file(update_state_dir().join(AUTO_UPDATE_SUPPRESS_FILE));
}

fn update_state_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("worktree-navigator");
    }
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".config/worktree-navigator"),
        Err(_) => PathBuf::from("."),
    }
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
