use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    app::{App, COMMANDS},
    types::{ActiveAction, CopySecretsPhase, SyncStatus},
};

pub fn draw(f: &mut Frame, app: &mut App) {
    app.item_rows.clear();

    let area = f.area();
    app.frame_width = area.width;
    app.frame_height = area.height;
    draw_panel(f, app, area);

    let show_sync_overlay = app.active_action == ActiveAction::SyncTrees
        && (app.sync_loading || !app.sync_results.is_empty());
    let show_delete_overlay =
        app.active_action == ActiveAction::Delete && (app.delete_confirming || app.delete_loading);
    let show_copy_overlay = app.active_action == ActiveAction::CopySecrets
        && (app.copy_secrets_phase == CopySecretsPhase::ConfirmOverwrite
            || app.copy_secrets_loading);

    match app.active_action {
        ActiveAction::NewBranch => draw_new_branch_overlay(f, app, area),
        ActiveAction::SyncPr => draw_sync_pr_overlay(f, app, area),
        ActiveAction::SyncTrees if show_sync_overlay => draw_sync_overlay(f, app, area),
        ActiveAction::Delete if show_delete_overlay => draw_delete_overlay(f, app, area),
        ActiveAction::CopySecrets if show_copy_overlay => draw_copy_secrets_overlay(f, app, area),
        ActiveAction::CloneRepo => draw_clone_overlay(f, app, area),
        _ => {}
    }

    // Error bar at the bottom (for errors after overlays close)
    let show_error_bar = app.active_action == ActiveAction::None
        || (app.active_action == ActiveAction::CopySecrets
            && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite);
    if show_error_bar {
        if let Some(err) = &app.overlay_error {
            let err_area = Rect {
                x: area.x + 2,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            };
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!(" ✗ {err} "),
                    Style::default().fg(Color::White).bg(Color::Red),
                )),
                err_area,
            );
        }
    }
}

// ─────────────────────────────── Main panel ─────────────────────────────────

fn draw_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let repo_name = app
        .repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?");

    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();
    let delete_select =
        app.active_action == ActiveAction::Delete && !app.delete_confirming && !app.delete_loading;
    let copy_select = app.active_action == ActiveAction::CopySecrets
        && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite
        && !app.copy_secrets_loading;
    let is_active =
        app.active_action == ActiveAction::None || sync_select || delete_select || copy_select;
    let border_color = if is_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(format!(" ⎇  Worktree Navigator — {repo_name} "))
        .title_alignment(Alignment::Center)
        .title_bottom(
            Line::from(Span::styled(
                format!(" v{} ", env!("CARGO_PKG_VERSION")),
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(Alignment::Right),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(COMMANDS.len() as u16 + 2), // "COMMANDS" header + items
            Constraint::Min(3),                            // "WORKTREES" header + list
            Constraint::Length(1),                         // help bar
        ])
        .split(inner);

    draw_commands(f, app, sections[0]);
    draw_worktrees(f, app, sections[1]);
    draw_help(f, app, sections[2]);
}

fn draw_commands(f: &mut Frame, app: &mut App, area: Rect) {
    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();
    let delete_select =
        app.active_action == ActiveAction::Delete && !app.delete_confirming && !app.delete_loading;
    let copy_select = app.active_action == ActiveAction::CopySecrets
        && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite
        && !app.copy_secrets_loading;
    let inline_select = sync_select || delete_select || copy_select;

    let header_style = if inline_select {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };

    f.render_widget(
        Paragraph::new(Line::from(Span::styled("COMMANDS", header_style))),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    for (i, command) in COMMANDS.iter().enumerate() {
        let row = area.y + 1 + i as u16;
        app.item_rows.push((row, i));

        let is_sync_cmd = command.action == ActiveAction::SyncTrees;
        let is_delete_cmd = command.action == ActiveAction::Delete;
        let is_copy_cmd = command.action == ActiveAction::CopySecrets;
        let focused_cmd = if sync_select {
            is_sync_cmd
        } else if delete_select {
            is_delete_cmd
        } else if copy_select {
            is_copy_cmd
        } else {
            false
        };

        let style = if inline_select {
            if focused_cmd {
                // Active inline-select indicator: underline the focused command.
                let color = if sync_select {
                    Color::Green
                } else if copy_select {
                    Color::Green
                } else {
                    Color::Red
                };
                Style::default()
                    .fg(color)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(Color::DarkGray)
            }
        } else {
            let selected = app.active_action == ActiveAction::None && app.selected_index == i;
            let hovered = app.active_action == ActiveAction::None
                && !selected
                && app.hovered_row == Some(row);
            if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if hovered {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }
        };

        let prefix =
            if !inline_select && app.active_action == ActiveAction::None && app.selected_index == i
            {
                "❯ "
            } else {
                "  "
            };

        let shortcut_style = if inline_select && !focused_cmd {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(command.label, style),
                Span::styled(format!(" [{}]", command.shortcut), shortcut_style),
            ])),
            Rect {
                x: area.x,
                y: row,
                width: area.width,
                height: 1,
            },
        );
    }
}

fn draw_worktrees(f: &mut Frame, app: &mut App, area: Rect) {
    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();
    let delete_select =
        app.active_action == ActiveAction::Delete && !app.delete_confirming && !app.delete_loading;
    let copy_select = app.active_action == ActiveAction::CopySecrets
        && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite
        && !app.copy_secrets_loading;

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("WORKTREES", header_style))),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    let list_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    if app.worktrees_loading {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  Loading…",
                Style::default().fg(Color::DarkGray),
            )),
            list_area,
        );
        return;
    }

    if let Some(err) = &app.worktrees_error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("  ✗ {err}"),
                Style::default().fg(Color::Red),
            )),
            list_area,
        );
        return;
    }

    if app.worktrees.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No worktrees found",
                Style::default().fg(Color::DarkGray),
            )),
            list_area,
        );
        return;
    }

    let max_rows = list_area.height as usize;
    let cmd_len = COMMANDS.len();

    let selected_delete_wt_idx = if delete_select {
        let deletable_indices: Vec<usize> = app
            .worktrees
            .iter()
            .enumerate()
            .filter_map(|(i, wt)| (!wt.is_main && !wt.is_current).then_some(i))
            .collect();
        deletable_indices
            .get(
                app.overlay_index
                    .min(deletable_indices.len().saturating_sub(1)),
            )
            .copied()
    } else {
        None
    };

    let selected_wt_idx = if sync_select {
        Some(app.sync_selected_idx)
    } else if copy_select {
        Some(
            if app.copy_secrets_phase == CopySecretsPhase::SelectSource {
                app.copy_secrets_source_idx.unwrap_or_else(|| {
                    app.worktrees
                        .iter()
                        .position(|wt| wt.is_current)
                        .unwrap_or(0)
                })
            } else {
                app.copy_secrets_target_idx
            },
        )
    } else if delete_select {
        selected_delete_wt_idx
    } else if app.active_action == ActiveAction::None && app.selected_index >= cmd_len {
        Some(app.selected_index - cmd_len)
    } else {
        None
    }
    .map(|idx| idx.min(app.worktrees.len().saturating_sub(1)));

    let start_idx = if app.worktrees.len() > max_rows {
        let sel = selected_wt_idx.unwrap_or(0);
        sel.saturating_sub(max_rows.saturating_sub(1))
            .min(app.worktrees.len() - max_rows)
    } else {
        0
    };

    for (visible_i, (i, wt)) in app
        .worktrees
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(max_rows)
        .enumerate()
    {
        let idx = cmd_len + i;
        let row = list_area.y + visible_i as u16;
        app.item_rows.push((row, idx));

        let selected = if sync_select {
            app.sync_selected_idx == i
        } else if copy_select {
            if app.copy_secrets_phase == CopySecretsPhase::SelectSource {
                app.copy_secrets_source_idx.unwrap_or_else(|| {
                    app.worktrees
                        .iter()
                        .position(|wt| wt.is_current)
                        .unwrap_or(0)
                }) == i
            } else {
                app.copy_secrets_target_idx == i
            }
        } else if delete_select {
            selected_delete_wt_idx == Some(i)
        } else {
            app.active_action == ActiveAction::None && app.selected_index == idx
        };

        let can_hover =
            app.active_action == ActiveAction::None || sync_select || delete_select || copy_select;
        let hovered = can_hover && !selected && app.hovered_row == Some(row);
        let deletable = !wt.is_main && !wt.is_current;
        let copy_disabled = app.copy_secrets_phase == CopySecretsPhase::SelectTarget
            && app.copy_secrets_source_idx == Some(i);

        let base_style = if selected {
            let selected_color = if delete_select {
                Color::Red
            } else if copy_select {
                Color::Green
            } else {
                Color::Cyan
            };
            Style::default()
                .fg(selected_color)
                .add_modifier(Modifier::BOLD)
        } else if copy_disabled {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM)
        } else if delete_select && !deletable {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM)
        } else if hovered {
            Style::default()
                .fg(if wt.is_main {
                    Color::Green
                } else if wt.is_current {
                    Color::Yellow
                } else {
                    Color::White
                })
                .add_modifier(Modifier::BOLD)
        } else if wt.is_main {
            Style::default().fg(Color::Green)
        } else if wt.is_current {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let mut spans = vec![
            Span::styled(if selected { "❯ " } else { "  " }, base_style),
            Span::styled(
                if copy_select {
                    if wt.has_secrets { "● " } else { "○ " }
                } else {
                    ""
                },
                Style::default().fg(Color::Green),
            ),
            Span::styled(wt.branch.clone(), base_style),
        ];

        let tag = match (wt.is_main, wt.is_current) {
            (true, true) => Some(" [default / current]"),
            (true, false) => Some(" [default]"),
            (false, true) => Some(" [current]"),
            (false, false) => None,
        };
        if let Some(t) = tag {
            spans.push(Span::styled(t, Style::default().fg(Color::DarkGray)));
        }

        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: list_area.x,
                y: row,
                width: list_area.width,
                height: 1,
            },
        );
    }
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();
    let delete_select =
        app.active_action == ActiveAction::Delete && !app.delete_confirming && !app.delete_loading;
    let copy_select = app.active_action == ActiveAction::CopySecrets
        && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite
        && !app.copy_secrets_loading;

    let text = if sync_select {
        Line::from(vec![
            Span::styled("↑↓/jk/click", Style::default().fg(Color::Green)),
            Span::styled(
                "  select branch to sync    ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("Enter/click", Style::default().fg(Color::Green)),
            Span::styled("  sync    ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().fg(Color::DarkGray)),
            Span::styled("  cancel", Style::default().fg(Color::DarkGray)),
        ])
    } else if delete_select {
        let has_deletable = app.worktrees.iter().any(|wt| !wt.is_main && !wt.is_current);
        if has_deletable {
            Line::from(vec![
                Span::styled("↑↓/jk/click", Style::default().fg(Color::Red)),
                Span::styled(
                    "  select worktree to delete    ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("Enter/click", Style::default().fg(Color::Red)),
                Span::styled("  confirm    ", Style::default().fg(Color::DarkGray)),
                Span::styled("Esc", Style::default().fg(Color::DarkGray)),
                Span::styled("  cancel", Style::default().fg(Color::DarkGray)),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    "No deletable worktrees",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("    ", Style::default().fg(Color::DarkGray)),
                Span::styled("Esc", Style::default().fg(Color::DarkGray)),
                Span::styled("  cancel", Style::default().fg(Color::DarkGray)),
            ])
        }
    } else if copy_select {
        let phase_text = if app.copy_secrets_phase == CopySecretsPhase::SelectSource {
            "  select source worktree    "
        } else {
            "  select destination worktree    "
        };
        Line::from(vec![
            Span::styled("●/○", Style::default().fg(Color::Green)),
            Span::styled(
                "  secrets present/empty    ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("↑↓/jk/click", Style::default().fg(Color::Green)),
            Span::styled(phase_text, Style::default().fg(Color::DarkGray)),
            Span::styled("Enter/click", Style::default().fg(Color::Green)),
            Span::styled("  confirm    ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().fg(Color::DarkGray)),
            Span::styled("  back/cancel", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled("↑↓/jk/scroll", Style::default().fg(Color::DarkGray)),
            Span::styled("  nav    ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter/click", Style::default().fg(Color::DarkGray)),
            Span::styled("  open    ", Style::default().fg(Color::DarkGray)),
            Span::styled("n  p  d  s  c", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "  branch/PR/delete/sync/copy    ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("q", Style::default().fg(Color::DarkGray)),
            Span::styled("  quit", Style::default().fg(Color::DarkGray)),
        ])
    };
    f.render_widget(Paragraph::new(text), area);
}

fn draw_copy_secrets_overlay(f: &mut Frame, app: &App, area: Rect) {
    if app.copy_secrets_loading {
        let popup = centered_rect(60, 5, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title(" Copy Secrets ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green));
        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!("⟳  Copying secrets{}", app.loading_animation_dots()),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "   This may take a moment.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            inner,
        );
        return;
    }

    let popup = centered_rect(60, 8, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Copy Secrets ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(popup).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, popup);

    let Some(source_idx) = app.copy_secrets_source_idx else {
        return;
    };
    let Some(source) = app.worktrees.get(source_idx) else {
        return;
    };
    let Some(target) = app.worktrees.get(app.copy_secrets_target_idx) else {
        return;
    };

    let yes_style = if app.copy_secrets_confirm_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    let no_style = if app.copy_secrets_confirm_yes {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    };

    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Overwrite secrets in ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    target.branch.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("?", Style::default().fg(Color::Yellow)),
            ]),
            Line::from(Span::styled(
                format!("Source: {}", source.branch),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![]),
            Line::from(vec![
                Span::styled("  Yes  ", yes_style),
                Span::styled("   ", Style::default()),
                Span::styled("  No  ", no_style),
            ]),
            Line::from(Span::styled(
                "Left/Right or click, Enter to confirm, Esc to cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ]),
        inner,
    );
}

// ─────────────────────────────── Overlays ───────────────────────────────────

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let w = r.width * percent_x / 100;
    Rect {
        x: r.x + r.width.saturating_sub(w) / 2,
        y: r.y + r.height.saturating_sub(height) / 2,
        width: w,
        height: height.min(r.height),
    }
}

fn draw_new_branch_overlay(f: &mut Frame, app: &App, area: Rect) {
    if app.new_branch_loading {
        let popup = centered_rect(60, 5, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title(" New Branch / Worktree ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);
        let branch = app
            .new_branch_pending
            .as_deref()
            .unwrap_or(app.input_buffer.trim());
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(
                        "⟳  Creating branch {}{}",
                        branch,
                        app.loading_animation_dots()
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "   This may take a moment.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            inner,
        );
        return;
    }

    let has_err = app.overlay_error.is_some();
    let popup = centered_rect(60, if has_err { 9 } else { 7 }, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" New Branch / Worktree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, popup);

    let mut constraints = vec![
        Constraint::Length(1), // input
        Constraint::Length(1), // spacer
        Constraint::Length(1), // hint
    ];
    if has_err {
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // error
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Branch name: ", Style::default().fg(Color::Gray)),
            Span::styled(
                &app.input_buffer,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ])),
        rows[0],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            "Enter to create  Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
        rows[2],
    );

    if let Some(err) = &app.overlay_error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(Color::Red),
            )),
            rows[4],
        );
    }
}

fn draw_sync_pr_overlay(f: &mut Frame, app: &App, area: Rect) {
    if app.sync_pr_loading {
        let popup = centered_rect(50, 7, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title(" Sync GitHub PR as Worktree ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta));
        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    "⟳  Fetching PR and creating worktree{}",
                    app.loading_animation_dots()
                ),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "   This may take a moment.",
                Style::default().fg(Color::DarkGray),
            )),
            rows[1],
        );
        if let Some(line) = app.sync_pr_output.last() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("   {line}"),
                    Style::default().fg(Color::DarkGray),
                )),
                rows[2],
            );
        }
        return;
    }

    let has_err = app.overlay_error.is_some();
    let popup = centered_rect(60, if has_err { 9 } else { 7 }, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Sync GitHub PR as Worktree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(popup).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, popup);

    let mut constraints = vec![
        Constraint::Length(1), // input
        Constraint::Length(1), // spacer
        Constraint::Length(1), // hint
    ];
    if has_err {
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // error
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("PR number: ", Style::default().fg(Color::Gray)),
            Span::styled(
                &app.input_buffer,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(Color::Magenta)),
        ])),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            "Use #123 or 123  Enter to checkout  Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
        rows[2],
    );

    if let Some(err) = &app.overlay_error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(Color::Red),
            )),
            rows[4],
        );
    }
}

fn draw_sync_overlay(f: &mut Frame, app: &App, area: Rect) {
    // Loading phase
    if app.sync_loading {
        let popup = centered_rect(50, 5, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title(" Sync Tree ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!("⟳  Fetching from remote{}", app.loading_animation_dots()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "   This may take a moment.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            inner,
        );
        return;
    }

    // Results phase
    if let Some(result) = app.sync_results.first() {
        let popup = centered_rect(60, 7, area);
        f.render_widget(Clear, popup);

        let fetch_label = if app.sync_fetch_ok {
            "fetch ✓"
        } else {
            "fetch ✗"
        };
        let fetch_color = if app.sync_fetch_ok {
            Color::Green
        } else {
            Color::Red
        };
        let block = Block::default()
            .title(format!(" Sync Result  {fetch_label} "))
            .title_style(Style::default().fg(fetch_color))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);

        let (icon, detail, color) = match &result.status {
            SyncStatus::UpToDate => ("✓", "Already up to date.".to_string(), Color::Green),
            SyncStatus::Updated(range) => ("↑", format!("Updated  {range}"), Color::Green),
            SyncStatus::Skipped(reason) => ("⚠", reason.clone(), Color::Yellow),
            SyncStatus::Error(msg) => {
                let short = msg
                    .lines()
                    .next()
                    .unwrap_or(msg)
                    .chars()
                    .take(70)
                    .collect::<String>();
                ("✗", short, Color::Red)
            }
        };

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{icon}  "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    result.branch.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("   {detail}"),
                Style::default().fg(color),
            )),
            rows[1],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Enter / Esc to close",
                Style::default().fg(Color::DarkGray),
            )),
            rows[2],
        );
    }
}

fn draw_delete_overlay(f: &mut Frame, app: &App, area: Rect) {
    if app.delete_loading {
        let popup = centered_rect(60, 5, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title(" Delete Worktree ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));
        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);
        let branch = app
            .delete_pending
            .as_deref()
            .and_then(|path| app.worktrees.iter().find(|wt| wt.path == path))
            .map(|wt| wt.branch.as_str())
            .unwrap_or("worktree");
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!("⟳  Deleting {}{}", branch, app.loading_animation_dots()),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "   This may take a moment.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            inner,
        );
        return;
    }

    let deletable = app.deletable_worktrees();
    if let Some(wt) = deletable.get(app.overlay_index) {
        let popup = centered_rect(60, 8, area);
        f.render_widget(Clear, popup);

        let block = Block::default()
            .title(" Delete Worktree ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "Delete worktree for branch:",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(Span::styled(
                    wt.branch.clone(),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    wt.path.clone(),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(vec![]),
                Line::from(Span::styled(
                    "Confirm? [Enter/y to delete, n/Esc to cancel]",
                    Style::default().fg(Color::Yellow),
                )),
            ]),
            inner,
        );
    }
}

fn draw_clone_overlay(f: &mut Frame, app: &App, area: Rect) {
    let has_err = app.clone_error.is_some();
    let height = if app.clone_loading {
        7
    } else if has_err {
        11
    } else {
        9
    };
    let popup = centered_rect(65, height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Clone Repository ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(popup).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, popup);

    if app.clone_loading {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new(Span::styled(
                format!("⟳  Cloning repository{}", app.loading_animation_dots()),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "   Working… this may take a moment.",
                Style::default().fg(Color::DarkGray),
            )),
            rows[1],
        );

        if let Some(line) = app.clone_output.last() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("   {line}"),
                    Style::default().fg(Color::DarkGray),
                )),
                rows[2],
            );
        }
        return;
    }

    let mut rows_constraints = vec![
        Constraint::Length(1), // URL line
        Constraint::Length(1), // spacer
        Constraint::Length(1), // dest/input line
        Constraint::Length(1), // spacer
        Constraint::Length(1), // hint
    ];
    if has_err {
        rows_constraints.push(Constraint::Length(1)); // spacer
        rows_constraints.push(Constraint::Length(1)); // error
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(rows_constraints)
        .split(inner);

    if app.clone_step == 0 {
        let before: String = app.input_buffer.chars().take(app.input_cursor).collect();
        let after: String = app.input_buffer.chars().skip(app.input_cursor).collect();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Repo:      ", Style::default().fg(Color::Gray)),
                Span::styled(
                    before,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(Color::Green)),
                Span::styled(
                    after,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Dest:      <cwd>/<repo-name>  (auto)",
                Style::default().fg(Color::DarkGray),
            )),
            rows[2],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Use URL or owner/repo  Enter to continue  Esc to quit",
                Style::default().fg(Color::DarkGray),
            )),
            rows[4],
        );
    } else {
        let before: String = app.input_buffer.chars().take(app.input_cursor).collect();
        let after: String = app.input_buffer.chars().skip(app.input_cursor).collect();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Repo:      ", Style::default().fg(Color::Gray)),
                Span::styled(&app.clone_url, Style::default().fg(Color::DarkGray)),
            ])),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Dest:      ", Style::default().fg(Color::Gray)),
                Span::styled(
                    before,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(Color::Green)),
                Span::styled(
                    after,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            rows[2],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Enter to clone  Esc to go back",
                Style::default().fg(Color::DarkGray),
            )),
            rows[4],
        );
    }

    if let Some(err) = &app.clone_error {
        let short = summarize_clone_error(err);
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("✗ {short}"),
                Style::default().fg(Color::Red),
            )),
            rows[6],
        );
    }
}

fn summarize_clone_error(err: &str) -> String {
    let lines: Vec<&str> = err
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return err.chars().take(80).collect();
    }

    let selected = lines
        .iter()
        .rev()
        .copied()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with("fatal:")
                || lower.starts_with("error:")
                || lower.contains("permission denied")
                || lower.contains("not found")
        })
        .or_else(|| {
            lines
                .iter()
                .copied()
                .find(|line| !line.starts_with("Cloning into "))
        })
        .unwrap_or(lines[0]);

    selected.chars().take(80).collect()
}
