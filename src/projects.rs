use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::store::State;
use crate::{git, store};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    #[serde(skip)]
    pub favorite_rank: Option<usize>,
}

/// Favorites first, in the order they were added, then the rest by name.
pub fn list_projects() -> Vec<Project> {
    let state = store::load().unwrap_or_default();
    let mut projects: Vec<Project> = state
        .projects
        .into_iter()
        .filter(|project| git::is_managed_workspace(&project.path))
        .map(|mut project| {
            project.favorite_rank = state
                .favorites
                .iter()
                .position(|favorite| *favorite == project.path);
            project
        })
        .collect();

    projects.sort_by(|a, b| {
        (a.favorite_rank.is_none(), a.favorite_rank, &a.name, &a.path).cmp(&(
            b.favorite_rank.is_none(),
            b.favorite_rank,
            &b.name,
            &b.path,
        ))
    });
    projects
}

pub fn toggle_favorite(path: &Path) -> Result<()> {
    let mut state = store::load().unwrap_or_default();
    if let Some(idx) = state.favorites.iter().position(|favorite| favorite == path) {
        state.favorites.remove(idx);
    } else {
        state.favorites.push(path.to_path_buf());
    }
    store::save(&state)
}

/// Renames the project folder, then relinks its worktrees and the saved state to the new path.
pub fn rename_project(project: &Project, new_name: &str) -> Result<PathBuf> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name == "." || new_name == ".." || new_name.contains(['/', '\\'])
    {
        bail!("'{new_name}' is not a valid folder name.");
    }

    let old_path = &project.path;
    let new_path = old_path.with_file_name(new_name);
    if new_path == *old_path {
        return Ok(new_path);
    }
    if new_path.exists() {
        bail!("{} already exists.", new_path.display());
    }

    fs::rename(old_path, &new_path).with_context(|| {
        format!(
            "Failed to move {} to {}",
            old_path.display(),
            new_path.display()
        )
    })?;

    let mut state = store::load().unwrap_or_default();
    move_project_in_state(&mut state, old_path, &new_path, new_name);
    store::save(&state)?;

    git::repair_moved_workspace(old_path, &new_path)?;
    Ok(new_path)
}

fn move_project_in_state(state: &mut State, old_path: &Path, new_path: &Path, new_name: &str) {
    let moved = |path: &Path| -> Option<PathBuf> {
        let rest = path.strip_prefix(old_path).ok()?;
        Some(if rest.as_os_str().is_empty() {
            new_path.to_path_buf()
        } else {
            new_path.join(rest)
        })
    };

    for project in &mut state.projects {
        if project.path == old_path {
            project.name = new_name.to_string();
        }
        if let Some(path) = moved(&project.path) {
            project.path = path;
        }
    }
    for favorite in &mut state.favorites {
        if let Some(path) = moved(favorite) {
            *favorite = path;
        }
    }
    state.worktrees = std::mem::take(&mut state.worktrees)
        .into_iter()
        .map(|(root, mut worktrees)| {
            for worktree in &mut worktrees {
                if let Some(path) = moved(Path::new(&worktree.path)) {
                    worktree.path = path.to_string_lossy().to_string();
                }
            }
            (moved(&root).unwrap_or(root), worktrees)
        })
        .collect();
}

pub fn register(workspace_root: &Path) {
    register_paths(&[workspace_root.to_path_buf()]);
}

pub fn register_all(workspace_roots: &[PathBuf]) {
    let paths: Vec<PathBuf> = workspace_roots
        .iter()
        .flat_map(|root| {
            let nested = git::list_child_projects(root);
            if nested.is_empty() {
                vec![root.clone()]
            } else {
                nested
            }
        })
        .collect();
    register_paths(&paths);
}

fn register_paths(workspace_roots: &[PathBuf]) {
    let mut state = store::load().unwrap_or_default();
    let fresh: Vec<Project> = workspace_roots
        .iter()
        .map(|root| root.canonicalize().unwrap_or_else(|_| root.clone()))
        .filter(|path| {
            !state.projects.iter().any(|project| &project.path == path)
                && git::is_managed_workspace(path)
                && !git::is_projects_container(path)
        })
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_string();
            Some(Project {
                name,
                path,
                favorite_rank: None,
            })
        })
        .collect();

    if !fresh.is_empty() {
        state.projects.extend(fresh);
        let _ = store::save(&state);
    }
}

pub fn find_project(name: &str) -> Result<Project> {
    let query = name.trim();
    let projects = list_projects();
    if projects.is_empty() {
        bail!("No projects registered yet. Run `wt clone <repo>` to create one.");
    }

    let matches = match_projects(&projects, query);
    match matches.len() {
        0 => bail!(
            "No project named '{query}'.\n\nAvailable projects:\n{}",
            format_project_list(projects.iter())
        ),
        1 => Ok(matches[0].clone()),
        _ => bail!(
            "'{query}' matches several projects:\n{}",
            format_project_list(matches.into_iter())
        ),
    }
}

/// Falls back to any worktree, so a project without its default branch checked out
/// still opens.
pub fn default_worktree_path(project: &Project) -> Result<PathBuf> {
    if let Some(main) = store::cached_worktrees(&project.path, &project.path)
        .unwrap_or_default()
        .into_iter()
        .find(|worktree| worktree.is_main && Path::new(&worktree.path).is_dir())
    {
        return Ok(PathBuf::from(main.path));
    }

    let mut worktrees = git::list_workspace_worktrees(&project.path).unwrap_or_default();
    if worktrees.is_empty() {
        worktrees = git::list_worktrees(&project.path).unwrap_or_default();
    }

    worktrees
        .iter()
        .find(|worktree| worktree.is_main)
        .or_else(|| worktrees.first())
        .cloned()
        .map(|worktree| PathBuf::from(worktree.path))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Project '{}' has no worktree to open ({}).",
                project.name,
                project.path.display()
            )
        })
}

pub fn format_project_list<'a>(projects: impl Iterator<Item = &'a Project>) -> String {
    projects
        .map(|project| format!("  {}  ({})", project.name, project.path.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exact name first, then case-insensitive, then substring, so an exact name is never
/// shadowed by a longer one.
fn match_projects<'a>(projects: &'a [Project], query: &str) -> Vec<&'a Project> {
    let lowered = query.to_lowercase();
    for candidates in [
        projects
            .iter()
            .filter(|project| project.name == query)
            .collect::<Vec<_>>(),
        projects
            .iter()
            .filter(|project| project.name.to_lowercase() == lowered)
            .collect(),
        projects
            .iter()
            .filter(|project| project.name.to_lowercase().contains(&lowered))
            .collect(),
    ] {
        if !candidates.is_empty() {
            return candidates;
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{Project, match_projects, move_project_in_state};
    use crate::store::State;
    use crate::types::Worktree;
    use std::path::{Path, PathBuf};

    fn project(name: &str) -> Project {
        Project {
            name: name.to_string(),
            path: PathBuf::from("/tmp").join(name),
            favorite_rank: None,
        }
    }

    #[test]
    fn exact_name_wins_over_substring_matches() {
        let projects = vec![project("api"), project("api-gateway")];

        let matches = match_projects(&projects, "api");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "api");
    }

    #[test]
    fn falls_back_to_case_insensitive_and_substring_matches() {
        let projects = vec![project("Worktree-Navigator"), project("other")];

        assert_eq!(match_projects(&projects, "worktree-navigator").len(), 1);
        assert_eq!(
            match_projects(&projects, "navigator")[0].name,
            projects[0].name
        );
        assert!(match_projects(&projects, "missing").is_empty());
    }

    #[test]
    fn ambiguous_queries_return_every_candidate() {
        let projects = vec![project("api-gateway"), project("api-worker")];

        assert_eq!(match_projects(&projects, "api").len(), 2);
    }

    #[test]
    fn moving_a_project_rewrites_its_paths_and_name_in_state() {
        let mut state = State {
            projects: vec![project("old"), project("other")],
            favorites: vec![PathBuf::from("/tmp/old")],
            ..State::default()
        };
        state.worktrees.insert(
            PathBuf::from("/tmp/old"),
            vec![Worktree {
                path: "/tmp/old/main".to_string(),
                branch: "main".to_string(),
                is_main: true,
                is_current: false,
                has_secrets: false,
            }],
        );

        move_project_in_state(
            &mut state,
            Path::new("/tmp/old"),
            Path::new("/tmp/new"),
            "new",
        );

        assert_eq!(state.projects[0].name, "new");
        assert_eq!(state.projects[0].path, PathBuf::from("/tmp/new"));
        assert_eq!(state.projects[1].path, PathBuf::from("/tmp/other"));
        assert_eq!(state.favorites, vec![PathBuf::from("/tmp/new")]);
        let worktrees = &state.worktrees[Path::new("/tmp/new")];
        assert_eq!(worktrees[0].path, "/tmp/new/main");
    }
}
