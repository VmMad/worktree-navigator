use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::{App, COMMANDS},
    types::{ActiveAction, CheckoutRemotePhase, CopySecretsPhase, OptionsPhase, SyncStatus},
    version,
};

const MIN_TERMINAL_WIDTH: u16 = 24;
const MIN_TERMINAL_HEIGHT: u16 = 8;

pub fn draw(f: &mut Frame, app: &mut App) {
    app.item_rows.clear();

    let area = f.area();
    app.frame_width = area.width;
    app.frame_height = area.height;

    if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
        draw_too_small(f, area);
        return;
    }

    draw_panel(f, app, area);

    let show_sync_overlay = app.active_action == ActiveAction::SyncTrees
        && (app.sync_loading || !app.sync_results.is_empty());
    let show_delete_overlay = app.active_action == ActiveAction::Delete
        && (app.delete_confirming || app.delete_warn_current || app.delete_loading);
    let show_copy_overlay = app.active_action == ActiveAction::CopySecrets
        && (app.copy_secrets_phase == CopySecretsPhase::ConfirmOverwrite
            || app.copy_secrets_loading);

    match app.active_action {
        ActiveAction::NewBranch => draw_new_branch_overlay(f, app, area),
        ActiveAction::Rename => draw_rename_overlay(f, app, area),
        ActiveAction::SyncPr => draw_sync_pr_overlay(f, app, area),
        ActiveAction::SyncTrees if show_sync_overlay => draw_sync_overlay(f, app, area),
        ActiveAction::Delete if show_delete_overlay => draw_delete_overlay(f, app, area),
        ActiveAction::CopySecrets if show_copy_overlay => draw_copy_secrets_overlay(f, app, area),
        ActiveAction::Options => draw_options_overlay(f, app, area),
        ActiveAction::CloneRepo => draw_clone_overlay(f, app, area),
        ActiveAction::CheckoutRemote => draw_checkout_remote_overlay(f, app, area),
        _ => {}
    }

    // Error bar at the bottom (for errors after overlays close)
    let show_error_bar = app.active_action == ActiveAction::None
        || (app.active_action == ActiveAction::CopySecrets
            && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite);
    if show_error_bar && let Some(err) = &app.overlay_error {
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

fn draw_too_small(f: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Terminal too small",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
            Line::from(Span::styled(
                "Resize to continue",
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(Alignment::Center),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false }),
        area,
    );
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
    let delete_select = app.active_action == ActiveAction::Delete
        && !app.delete_confirming
        && !app.delete_warn_current
        && !app.delete_loading;
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
                format!(" v{} ", version::current_version()),
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(Alignment::Right),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

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
    if area.width == 0 || area.height == 0 {
        return;
    }

    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();
    let delete_select = app.active_action == ActiveAction::Delete
        && !app.delete_confirming
        && !app.delete_warn_current
        && !app.delete_loading;
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

    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("COMMANDS", header_style))),
        header_area,
    );

    let visible_rows = area.height.saturating_sub(1) as usize;
    if visible_rows == 0 {
        return;
    }

    for (i, command) in COMMANDS.iter().take(visible_rows).enumerate() {
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
                let color = if delete_select {
                    Color::Red
                } else {
                    Color::Green
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
    if area.width == 0 || area.height == 0 {
        return;
    }

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
        if app.is_deletable_worktree_idx(app.overlay_index) {
            Some(app.overlay_index)
        } else {
            app.first_deletable_worktree_idx()
        }
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
        let deletable = !wt.is_main;
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
        } else if copy_disabled || (delete_select && !deletable) {
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
                } else if delete_select && deletable {
                    if app.delete_checked.contains(&i) {
                        "● "
                    } else {
                        "○ "
                    }
                } else {
                    ""
                },
                Style::default().fg(if delete_select {
                    Color::Red
                } else {
                    Color::Green
                }),
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
    if area.width == 0 || area.height == 0 {
        return;
    }

    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();
    let delete_select = app.active_action == ActiveAction::Delete
        && !app.delete_confirming
        && !app.delete_warn_current
        && !app.delete_loading;
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
        let has_deletable = app.worktrees.iter().any(|wt| !wt.is_main);
        if has_deletable {
            Line::from(vec![
                Span::styled("●/○", Style::default().fg(Color::Red)),
                Span::styled(
                    "  selected/unselected    ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("↑↓/jk/click", Style::default().fg(Color::Red)),
                Span::styled("  move cursor    ", Style::default().fg(Color::DarkGray)),
                Span::styled("Space/click", Style::default().fg(Color::Red)),
                Span::styled("  toggle    ", Style::default().fg(Color::DarkGray)),
                Span::styled("Enter", Style::default().fg(Color::Red)),
                Span::styled("  delete/confirm    ", Style::default().fg(Color::DarkGray)),
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
            Span::styled(
                "b  m  p  d  s  c  o  r",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "  branch/rename/PR/delete/sync/copy/options/remote    ",
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
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let no_label = if app.copy_secrets_confirm_yes {
        "  No  "
    } else {
        "[ No ]"
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
                Span::styled(no_label, no_style),
            ]),
            Line::from(Span::styled(
                "Left/Right or click, Enter to confirm, Esc to cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ]),
        inner,
    );
}

fn draw_options_overlay(f: &mut Frame, app: &App, area: Rect) {
    let has_err = app.overlay_error.is_some();
    let is_editing = app.options_phase == OptionsPhase::Editing;
    let item_count = app.repo_config.post_create_scripts.len();
    let list_height = item_count.min(6) as u16;
    let popup_height = if is_editing {
        10 + u16::from(has_err) * 2
    } else {
        8 + list_height + u16::from(has_err) * 2
    };
    let popup = centered_rect(74, popup_height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Options ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(popup).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, popup);

    if is_editing {
        let mut constraints = vec![
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ];
        if has_err {
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(1));
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let editing_label = if app.options_edit_idx.is_some() {
            "Edit a shell command to run after creating a worktree."
        } else {
            "Add a shell command to run after creating a worktree."
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "Post-create worktree scripts",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    editing_label,
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            rows[0],
        );

        let (before, after) = app.input_parts();
        f.render_widget(
            Paragraph::new(input_line("Command: ", before, after, Color::Yellow)),
            rows[1],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "Enter to save  Esc to cancel",
                Style::default().fg(Color::DarkGray),
            )),
            rows[3],
        );

        if let Some(err) = &app.overlay_error {
            let err_row = rows.len() - 1;
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("✗ {err}"),
                    Style::default().fg(Color::Red),
                )),
                rows[err_row],
            );
        }
        return;
    }

    let mut body_rows = vec![
        Line::from(Span::styled(
            "Enabled commands run automatically after a new worktree is created.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "Commands run inside the new worktree with WT_* paths available.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(vec![]),
    ];

    if app.repo_config.post_create_scripts.is_empty() {
        body_rows.push(Line::from(Span::styled(
            "No scripts configured yet. Press a to add one.",
            Style::default().fg(Color::Yellow),
        )));
    } else {
        let window_size = 6usize;
        let total = app.repo_config.post_create_scripts.len();
        let selected = app.options_selected_idx.min(total.saturating_sub(1));
        let start = selected.saturating_sub(window_size.saturating_sub(1));
        let end = (start + window_size).min(total);
        let start = end.saturating_sub(window_size);

        if start > 0 {
            body_rows.push(Line::from(Span::styled(
                format!("... {start} earlier"),
                Style::default().fg(Color::DarkGray),
            )));
        }

        for (idx, script) in app
            .repo_config
            .post_create_scripts
            .iter()
            .enumerate()
            .skip(start)
            .take(end - start)
        {
            let is_selected = idx == app.options_selected_idx;
            let marker = if script.enabled { "[x]" } else { "[ ]" };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if script.enabled {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            body_rows.push(Line::from(Span::styled(
                format!("{marker} {}", script.command),
                style,
            )));
        }

        if end < total {
            body_rows.push(Line::from(Span::styled(
                format!("... {} more", total - end),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    body_rows.push(Line::from(vec![]));
    body_rows.push(Line::from(Span::styled(
        "a add  e/Enter edit  Space toggle  d delete  Esc close",
        Style::default().fg(Color::DarkGray),
    )));

    if let Some(err) = &app.overlay_error {
        body_rows.push(Line::from(vec![]));
        body_rows.push(Line::from(Span::styled(
            format!("✗ {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    f.render_widget(Paragraph::new(body_rows).wrap(Wrap { trim: false }), inner);
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

fn input_line(label: &str, before: String, after: String, caret_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(label.to_string(), Style::default().fg(Color::Gray)),
        Span::styled(
            before,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("█", Style::default().fg(caret_color)),
        Span::styled(
            after,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ])
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
            .unwrap_or_else(|| app.input_buffer.trim());
        let loading_label = if app.new_branch_use_existing {
            format!(
                "⟳  Creating worktree from {}{}",
                branch,
                app.loading_animation_dots()
            )
        } else {
            format!(
                "⟳  Creating branch {}{}",
                branch,
                app.loading_animation_dots()
            )
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    loading_label,
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

    if let Some(branch) = &app.new_branch_confirm_existing {
        let popup = centered_rect(48, 10, area);
        f.render_widget(Clear, popup);

        let block = Block::default()
            .title(" Existing Branch ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);

        let yes_style = if app.new_branch_confirm_yes {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let no_style = if app.new_branch_confirm_yes {
            Style::default().fg(Color::White)
        } else {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD)
        };
        let no_label = if app.new_branch_confirm_yes {
            "  No  "
        } else {
            "[ No ]"
        };

        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "Branch already exists",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "Create a new worktree from it instead?",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(vec![]),
                Line::from(Span::styled(
                    branch.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![]),
                Line::from(vec![
                    Span::styled("  Yes  ", yes_style),
                    Span::styled("   ", Style::default()),
                    Span::styled(no_label, no_style),
                ])
                .alignment(Alignment::Center),
                Line::from(Span::styled(
                    "Use arrows, Enter, or Esc",
                    Style::default().fg(Color::DarkGray),
                ))
                .alignment(Alignment::Center),
            ]),
            inner,
        );
        return;
    }

    let has_err = app.overlay_error.is_some();
    let has_base = app.new_branch_base.is_some();
    let mut height = 7;
    if has_err {
        height += 2;
    }
    if has_base {
        height += 1;
    }

    let popup = centered_rect(60, height, area);
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
    ];
    if has_base {
        constraints.push(Constraint::Length(1)); // base branch
    }
    constraints.push(Constraint::Length(1)); // spacer
    constraints.push(Constraint::Length(1)); // hint

    if has_err {
        constraints.push(Constraint::Length(1)); // spacer
        constraints.push(Constraint::Length(1)); // error
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut row_idx = 0;
    let (before, after) = app.input_parts();
    f.render_widget(
        Paragraph::new(input_line("Branch name: ", before, after, Color::Yellow)),
        rows[row_idx],
    );
    row_idx += 1;

    if let Some(base) = &app.new_branch_base {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Base: ", Style::default().fg(Color::Gray)),
                Span::styled(base, Style::default().fg(Color::Cyan)),
            ])),
            rows[row_idx],
        );
        row_idx += 1;
    }

    row_idx += 1; // spacer
    f.render_widget(
        Paragraph::new(Span::styled(
            "Enter to create  Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
        rows[row_idx],
    );
    row_idx += 1;

    if let Some(err) = &app.overlay_error {
        row_idx += 1; // spacer
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(Color::Red),
            )),
            rows[row_idx],
        );
    }
}

fn draw_rename_overlay(f: &mut Frame, app: &App, area: Rect) {
    if app.rename_loading {
        let popup = centered_rect(60, 5, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title(" Rename Worktree ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue));
        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);
        let branch = app.input_buffer.trim();
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(
                        "⟳  Renaming branch to {}{}",
                        branch,
                        app.loading_animation_dots()
                    ),
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "   Updating branch and worktree path.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            inner,
        );
        return;
    }

    let target = app.rename_target_idx.and_then(|idx| app.worktrees.get(idx));
    let has_err = app.overlay_error.is_some();
    let mut height = 9;
    if has_err {
        height += 2;
    }

    let popup = centered_rect(60, height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Rename Worktree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(popup).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, popup);

    let mut constraints = vec![
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ];
    if has_err {
        constraints.push(Constraint::Length(1));
        constraints.push(Constraint::Min(1));
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let (before, after) = app.input_parts();
    f.render_widget(
        Paragraph::new(input_line("New branch: ", before, after, Color::Blue)),
        rows[0],
    );

    if let Some(target) = target {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Current: ", Style::default().fg(Color::Gray)),
                Span::styled(&target.branch, Style::default().fg(Color::Cyan)),
            ])),
            rows[1],
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                target.path.clone(),
                Style::default().fg(Color::DarkGray),
            )),
            rows[2],
        );
    }

    f.render_widget(
        Paragraph::new(Span::styled(
            "Enter to rename  Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
        rows[3],
    );

    if let Some(err) = &app.overlay_error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(Color::Red),
            ))
            .wrap(Wrap { trim: false }),
            rows[5],
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
                    "⟳  Fetching PR and preparing worktree{}",
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
    let popup = centered_rect(60, if has_err { 11 } else { 7 }, area);
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
        constraints.push(Constraint::Min(1)); // error (wraps)
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let (before, after) = app.input_parts();
    f.render_widget(
        Paragraph::new(input_line("PR number: ", before, after, Color::Magenta)),
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
            ))
            .wrap(Wrap { trim: false }),
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
            .and_then(|paths| {
                if paths.len() == 1 {
                    paths
                        .first()
                        .and_then(|path| app.worktrees.iter().find(|wt| wt.path == path.as_str()))
                        .map(|wt| wt.branch.clone())
                } else if paths.is_empty() {
                    None
                } else {
                    Some(format!("{} worktrees", paths.len()))
                }
            })
            .unwrap_or_else(|| "worktree".to_string());
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

    if !app.delete_confirm_targets.is_empty() {
        let popup = centered_rect(
            48,
            if app.delete_warn_current {
                12
            } else if app.delete_confirm_targets.len() > 1 {
                13
            } else {
                11
            },
            area,
        );
        f.render_widget(Clear, popup);

        let block_title = if app.delete_warn_current {
            " Delete Current Worktree "
        } else if app.delete_confirm_targets.len() > 1 {
            " Delete Worktrees "
        } else {
            " Delete Worktree "
        };
        let block = Block::default()
            .title(block_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let inner = block.inner(popup).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        f.render_widget(block, popup);

        let branches: Vec<String> = app
            .delete_confirm_targets
            .iter()
            .copied()
            .filter_map(|idx| app.worktrees.get(idx).map(|wt| wt.branch.clone()))
            .collect();
        let paths: Vec<String> = app
            .delete_confirm_targets
            .iter()
            .copied()
            .filter_map(|idx| app.worktrees.get(idx).map(|wt| wt.path.clone()))
            .collect();
        let current_branch = app.delete_confirm_targets.iter().copied().find_map(|idx| {
            app.worktrees
                .get(idx)
                .and_then(|wt| wt.is_current.then_some(wt.branch.clone()))
        });
        let fallback_branch = app
            .default_worktree_idx()
            .and_then(|idx| app.worktrees.get(idx).map(|wt| wt.branch.clone()));

        let headline = if app.delete_warn_current {
            "Current worktree selected".to_string()
        } else if branches.len() == 1 {
            "Delete this worktree?".to_string()
        } else {
            format!("Delete {} worktrees?", branches.len())
        };
        let detail = if app.delete_warn_current {
            current_branch.unwrap_or_else(|| "current worktree".to_string())
        } else if branches.len() == 1 {
            branches.first().cloned().unwrap_or_default()
        } else {
            String::new()
        };
        let supporting_line = if app.delete_warn_current {
            fallback_branch.map_or_else(
                || "wt will switch to the default worktree after deletion".to_string(),
                |branch| format!("wt will switch to {branch} after deletion"),
            )
        } else if paths.len() == 1 {
            paths.first().cloned().unwrap_or_default()
        } else {
            format!("Selected: {}", paths.len())
        };
        let warning_line = if app.delete_warn_current {
            "The current worktree is selected. Uncommitted changes might be lost."
        } else {
            "This cannot be undone."
        };

        let yes_style = if app.delete_confirm_yes {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let no_style = if app.delete_confirm_yes {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        };
        let no_label = if app.delete_confirm_yes {
            "  No  "
        } else {
            "[ No ]"
        };

        let mut body_lines = vec![
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                warning_line,
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![]),
        ];

        if app.delete_warn_current || branches.len() == 1 {
            body_lines.push(Line::from(Span::styled(
                detail,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            body_lines.push(Line::from(Span::styled(
                supporting_line,
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for branch in branches.iter().take(3) {
                body_lines.push(Line::from(vec![
                    Span::styled("• ", Style::default().fg(Color::Red)),
                    Span::styled(
                        branch.clone(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            if branches.len() > 3 {
                body_lines.push(Line::from(Span::styled(
                    format!("... and {} more", branches.len() - 3),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }

        if inner.height == 0 {
            return;
        }

        let footer_height = if inner.height >= 2 { 2 } else { 1 };
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
            .split(inner);
        let body_area = sections[0];
        let footer_area = sections[1];

        if body_area.height > 0 {
            f.render_widget(
                Paragraph::new(body_lines).wrap(Wrap { trim: false }),
                body_area,
            );
        }

        let footer_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(if footer_height == 2 {
                vec![Constraint::Length(1), Constraint::Length(1)]
            } else {
                vec![Constraint::Length(1)]
            })
            .split(footer_area);

        f.render_widget(
            Paragraph::new(
                Line::from(vec![
                    Span::styled("  Yes  ", yes_style),
                    Span::styled("   ", Style::default()),
                    Span::styled(no_label, no_style),
                ])
                .alignment(Alignment::Center),
            ),
            footer_rows[0],
        );

        if footer_height == 2 {
            f.render_widget(
                Paragraph::new(
                    Line::from(Span::styled(
                        "Use arrows, Enter, or Esc",
                        Style::default().fg(Color::DarkGray),
                    ))
                    .alignment(Alignment::Center),
                ),
                footer_rows[1],
            );
        }
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
            .constraints([Constraint::Length(1), Constraint::Length(1)])
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

    let (before, after) = app.input_parts();
    if app.clone_step == 0 {
        f.render_widget(
            Paragraph::new(input_line("Repo:      ", before, after, Color::Green)),
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
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Repo:      ", Style::default().fg(Color::Gray)),
                Span::styled(&app.clone_url, Style::default().fg(Color::DarkGray)),
            ])),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(input_line("Dest:      ", before, after, Color::Green)),
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

fn draw_checkout_remote_overlay(f: &mut Frame, app: &App, area: Rect) {
    const COLOR: Color = Color::Blue;

    match app.checkout_remote_phase {
        CheckoutRemotePhase::FetchingRemote => {
            let popup = centered_rect(60, 7, area);
            f.render_widget(Clear, popup);
            let block = Block::default()
                .title(" Checkout Remote Branch ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR));
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
                Paragraph::new(Line::from(vec![
                    Span::styled("Remote:  ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        &app.checkout_remote_name,
                        Style::default().fg(Color::DarkGray),
                    ),
                ])),
                rows[0],
            );
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!(
                        "⟳  Fetching {}{}",
                        app.checkout_remote_name,
                        app.loading_animation_dots()
                    ),
                    Style::default().fg(COLOR).add_modifier(Modifier::BOLD),
                )),
                rows[1],
            );
        }
        CheckoutRemotePhase::CreatingWorktree => {
            let popup = centered_rect(60, 5, area);
            f.render_widget(Clear, popup);
            let block = Block::default()
                .title(" Checkout Remote Branch ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR));
            let inner = block.inner(popup).inner(Margin {
                horizontal: 1,
                vertical: 1,
            });
            f.render_widget(block, popup);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        format!("⟳  Creating worktree{}", app.loading_animation_dots()),
                        Style::default().fg(COLOR).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        "   This may take a moment.",
                        Style::default().fg(Color::DarkGray),
                    )),
                ]),
                inner,
            );
        }
        CheckoutRemotePhase::SelectRemote | CheckoutRemotePhase::EnterBranch => {
            let has_err = app.overlay_error.is_some();
            let height = if has_err { 10 } else { 8 };
            let popup = centered_rect(60, height, area);
            f.render_widget(Clear, popup);
            let block = Block::default()
                .title(" Checkout Remote Branch ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR));
            let inner = block.inner(popup).inner(Margin {
                horizontal: 1,
                vertical: 1,
            });
            f.render_widget(block, popup);

            let mut constraints = vec![
                Constraint::Length(1), // remote line
                Constraint::Length(1), // branch line
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

            let (before, after) = app.input_parts();
            if app.checkout_remote_phase == CheckoutRemotePhase::SelectRemote {
                f.render_widget(
                    Paragraph::new(input_line("Remote:  ", before, after, COLOR)),
                    rows[0],
                );
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Branch:  ", Style::default().fg(Color::Gray)),
                        Span::styled("(enter remote first)", Style::default().fg(Color::DarkGray)),
                    ])),
                    rows[1],
                );
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "Enter to fetch  Esc to cancel",
                        Style::default().fg(Color::DarkGray),
                    )),
                    rows[3],
                );
            } else {
                let ghost = app.checkout_remote_ghost().unwrap_or_default();
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Remote:  ", Style::default().fg(Color::Gray)),
                        Span::styled(
                            &app.checkout_remote_name,
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled("  ✓", Style::default().fg(Color::Green)),
                    ])),
                    rows[0],
                );
                let mut branch_spans = input_line("Branch:  ", before, after, COLOR).spans;
                if !ghost.is_empty() {
                    branch_spans.push(Span::styled(ghost, Style::default().fg(Color::DarkGray)));
                }
                f.render_widget(Paragraph::new(Line::from(branch_spans)), rows[1]);
                let hint = if app.checkout_remote_ghost().is_some() {
                    "Tab to complete  Enter to checkout  Esc to go back"
                } else {
                    "Enter to checkout  Esc to go back"
                };
                f.render_widget(
                    Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))),
                    rows[3],
                );
            }

            if let Some(err) = &app.overlay_error {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!("✗ {err}"),
                        Style::default().fg(Color::Red),
                    )),
                    rows[5],
                );
            }
        }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::{Terminal, backend::TestBackend};

    use crate::{
        app::App,
        types::{ActiveAction, Worktree},
    };

    use super::{draw, draw_delete_overlay};

    #[test]
    fn delete_overlay_keeps_actions_visible_on_small_screens() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal should be created");
        let mut app = App::new(PathBuf::from("/tmp/repo"));
        app.worktrees = vec![
            Worktree {
                path: "/tmp/repo/main".to_string(),
                branch: "main".to_string(),
                is_main: true,
                is_current: true,
                has_secrets: false,
            },
            Worktree {
                path: "/tmp/repo/feature-small".to_string(),
                branch: "feature/small".to_string(),
                is_main: false,
                is_current: false,
                has_secrets: false,
            },
        ];
        app.active_action = ActiveAction::Delete;
        app.delete_confirm_targets = vec![1];
        app.delete_confirm_yes = true;

        terminal
            .draw(|frame| draw_delete_overlay(frame, &app, frame.area()))
            .expect("delete overlay should render");

        let buffer = terminal.backend().buffer();
        let visible_text = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();

        assert!(visible_text.contains("Yes"));
        assert!(visible_text.contains("No"));
    }

    #[test]
    fn full_ui_renders_without_overflow_on_small_screens() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal should be created");
        let mut app = App::new(PathBuf::from("/tmp/repo"));
        app.worktrees = vec![Worktree {
            path: "/tmp/repo/main".to_string(),
            branch: "main".to_string(),
            is_main: true,
            is_current: true,
            has_secrets: false,
        }];
        app.worktrees_loading = false;

        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("full ui should render on small screens");
    }

    #[test]
    fn tiny_terminal_renders_fallback_without_panicking() {
        let backend = TestBackend::new(12, 4);
        let mut terminal = Terminal::new(backend).expect("test terminal should be created");
        let mut app = App::new(PathBuf::from("/tmp/repo"));

        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("tiny terminal should render fallback");
    }
}
