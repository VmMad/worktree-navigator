use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{git, store};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
}

pub fn list_projects() -> Vec<Project> {
    let mut projects: Vec<Project> = store::load()
        .unwrap_or_default()
        .projects
        .into_iter()
        .filter(|project| git::is_managed_workspace(&project.path))
        .collect();

    projects.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    projects
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
            Some(Project { name, path })
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
    use super::{Project, match_projects};
    use std::path::PathBuf;

    fn project(name: &str) -> Project {
        Project {
            name: name.to_string(),
            path: PathBuf::from("/tmp").join(name),
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
}
