use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedArgs {
    Tui {
        mark_tree: bool,
    },
    Version,
    Update,
    Help,
    CheckoutPr {
        pr_number: u32,
    },
    Branch {
        branch_name: String,
        base: BranchBase,
    },
    Delete {
        branch_name: Option<String>,
        yes: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchBase {
    Auto,
    Current,
    Default,
    Explicit(String),
}

pub fn parse_args<I>(args: I) -> Result<ParsedArgs>
where
    I: IntoIterator<Item = String>,
{
    let mut args: Vec<String> = args.into_iter().collect();
    if args.is_empty() {
        return Ok(ParsedArgs::Tui { mark_tree: false });
    }

    if args.len() == 1 {
        match args[0].as_str() {
            "--version" | "-V" => return Ok(ParsedArgs::Version),
            "--update" => return Ok(ParsedArgs::Update),
            "--mark-tree" => return Ok(ParsedArgs::Tui { mark_tree: true }),
            "--help" | "-h" | "help" => return Ok(ParsedArgs::Help),
            _ => {}
        }
    }

    let command = args.remove(0);
    match command.as_str() {
        "pr" | "checkout-pr" => parse_pr(args),
        "b" | "branch" => parse_branch(args),
        "d" | "delete" => parse_delete(args),
        "--help" | "-h" | "help" => Ok(ParsedArgs::Help),
        _ => bail!("Unknown command '{command}'. Run `wt --help` for usage."),
    }
}

fn parse_pr(args: Vec<String>) -> Result<ParsedArgs> {
    if args.len() != 1 {
        bail!("Usage: wt pr <number>");
    }

    let raw = args[0].trim();
    let number = raw.trim_start_matches('#');
    if number.is_empty() {
        bail!("PR number cannot be empty.");
    }

    let pr_number = number
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("Invalid PR number '{raw}'."))?;
    if pr_number == 0 {
        bail!("PR number must be greater than zero.");
    }

    Ok(ParsedArgs::CheckoutPr { pr_number })
}

fn parse_branch(args: Vec<String>) -> Result<ParsedArgs> {
    let mut branch_name = None;
    let mut base = BranchBase::Auto;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == "--from-default" || arg == "-d" {
            ensure_auto_base(&base, "--from-default")?;
            base = BranchBase::Default;
            continue;
        }
        if arg == "--from-current" {
            ensure_auto_base(&base, "--from-current")?;
            base = BranchBase::Current;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--base=") {
            ensure_auto_base(&base, "--base")?;
            if value.trim().is_empty() {
                bail!("`--base` requires a branch name.");
            }
            base = BranchBase::Explicit(value.to_string());
            continue;
        }
        if arg == "--base" {
            ensure_auto_base(&base, "--base")?;
            let Some(value) = iter.next() else {
                bail!("`--base` requires a branch name.");
            };
            if value.trim().is_empty() {
                bail!("`--base` requires a branch name.");
            }
            base = BranchBase::Explicit(value);
            continue;
        }
        if arg.starts_with('-') {
            bail!("Unknown option '{arg}' for `wt branch`.");
        }
        if branch_name.is_some() {
            bail!("Usage: wt b <branch> [--from-default|--from-current|--base <branch>]");
        }
        branch_name = Some(arg);
    }

    let Some(branch_name) = branch_name else {
        bail!("Usage: wt b <branch> [--from-default|--from-current|--base <branch>]");
    };

    Ok(ParsedArgs::Branch { branch_name, base })
}

fn parse_delete(args: Vec<String>) -> Result<ParsedArgs> {
    let mut branch_name = None;
    let mut yes = false;

    for arg in args {
        if arg == "--yes" {
            yes = true;
            continue;
        }
        if arg.starts_with('-') {
            bail!("Unknown option '{arg}' for `wt delete`.");
        }
        if branch_name.is_some() {
            bail!("Usage: wt d [branch] [--yes]");
        }
        branch_name = Some(arg);
    }

    Ok(ParsedArgs::Delete { branch_name, yes })
}

fn ensure_auto_base(base: &BranchBase, flag: &str) -> Result<()> {
    if !matches!(base, BranchBase::Auto) {
        bail!("Conflicting base options; cannot combine `{flag}` with another base selector.");
    }
    Ok(())
}

pub fn print_help() {
    println!(
        "\
wt opens the interactive worktree UI by default.

Usage:
  wt
  wt pr <number>
  wt b <branch> [--from-default|--from-current|--base <branch>]
  wt d [branch] [--yes]

Aliases:
  wt checkout-pr <number>
  wt branch <branch> [...]
  wt delete [branch] [--yes]

Existing flags:
  wt --mark-tree
  wt --update
  wt --version
"
    );
}

#[cfg(test)]
mod tests {
    use super::{BranchBase, ParsedArgs, parse_args};

    fn parse(input: &[&str]) -> ParsedArgs {
        parse_args(input.iter().map(|value| value.to_string())).expect("args should parse")
    }

    #[test]
    fn defaults_to_tui_without_args() {
        assert_eq!(parse(&[]), ParsedArgs::Tui { mark_tree: false });
    }

    #[test]
    fn parses_legacy_flags() {
        assert_eq!(parse(&["--version"]), ParsedArgs::Version);
        assert_eq!(parse(&["-V"]), ParsedArgs::Version);
        assert_eq!(parse(&["--update"]), ParsedArgs::Update);
        assert_eq!(parse(&["--mark-tree"]), ParsedArgs::Tui { mark_tree: true });
    }

    #[test]
    fn parses_pr_aliases_and_hash_prefix() {
        assert_eq!(
            parse(&["pr", "123"]),
            ParsedArgs::CheckoutPr { pr_number: 123 }
        );
        assert_eq!(
            parse(&["checkout-pr", "#42"]),
            ParsedArgs::CheckoutPr { pr_number: 42 }
        );
    }

    #[test]
    fn parses_branch_defaults_and_base_flags() {
        assert_eq!(
            parse(&["b", "feat/test"]),
            ParsedArgs::Branch {
                branch_name: "feat/test".to_string(),
                base: BranchBase::Auto,
            }
        );
        assert_eq!(
            parse(&["branch", "feat/test", "--from-default"]),
            ParsedArgs::Branch {
                branch_name: "feat/test".to_string(),
                base: BranchBase::Default,
            }
        );
        assert_eq!(
            parse(&["b", "feat/test", "-d"]),
            ParsedArgs::Branch {
                branch_name: "feat/test".to_string(),
                base: BranchBase::Default,
            }
        );
        assert_eq!(
            parse(&["b", "feat/test", "--from-current"]),
            ParsedArgs::Branch {
                branch_name: "feat/test".to_string(),
                base: BranchBase::Current,
            }
        );
        assert_eq!(
            parse(&["b", "feat/test", "--base", "main"]),
            ParsedArgs::Branch {
                branch_name: "feat/test".to_string(),
                base: BranchBase::Explicit("main".to_string()),
            }
        );
        assert_eq!(
            parse(&["b", "feat/test", "--base=release/1"]),
            ParsedArgs::Branch {
                branch_name: "feat/test".to_string(),
                base: BranchBase::Explicit("release/1".to_string()),
            }
        );
    }

    #[test]
    fn parses_delete_defaults_and_yes_flag() {
        assert_eq!(
            parse(&["d"]),
            ParsedArgs::Delete {
                branch_name: None,
                yes: false,
            }
        );
        assert_eq!(
            parse(&["delete", "feat/test", "--yes"]),
            ParsedArgs::Delete {
                branch_name: Some("feat/test".to_string()),
                yes: true,
            }
        );
    }

    #[test]
    fn rejects_conflicting_branch_base_flags() {
        let err =
            parse_args(["b", "feat/test", "--from-default", "--base", "main"].map(str::to_string))
                .expect_err("conflicting flags should fail");
        assert_eq!(
            err.to_string(),
            "Conflicting base options; cannot combine `--base` with another base selector."
        );
    }
}
