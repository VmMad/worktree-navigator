use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, COMMANDS},
    types::{ActiveAction, ActivePanel, MessageKind},
};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // ── 30/70 horizontal split ──────────────────────────────────────────────
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    draw_sidebar(f, app, chunks[0]);
    draw_console(f, app, chunks[1]);

    // ── overlays (rendered on top) ──────────────────────────────────────────
    match app.active_action {
        ActiveAction::NewBranch => draw_new_branch_overlay(f, app, area),
        ActiveAction::SyncPr => draw_sync_pr_overlay(f, app, area),
        ActiveAction::Delete => draw_delete_overlay(f, app, area),
        ActiveAction::None => {}
    }
}

// ─────────────────────────────────── Sidebar ────────────────────────────────

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.active_panel == ActivePanel::Sidebar && app.active_action == ActiveAction::None;
    let border_color = if is_focused { Color::Cyan } else { Color::DarkGray };

    let block = Block::default()
        .title(" ⎇  Worktree Navigator ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(COMMANDS.len() as u16 + 2), // commands header + items
            Constraint::Min(4),                             // worktrees
            Constraint::Length(2),                         // help line
        ])
        .split(inner);

    draw_commands(f, app, sections[0]);
    draw_worktrees(f, app, sections[1]);
    draw_sidebar_help(f, sections[2]);
}

fn draw_commands(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.active_panel == ActivePanel::Sidebar
        && app.active_action == ActiveAction::None
        && app.sidebar_index < COMMANDS.len();

    let items: Vec<ListItem> = COMMANDS
        .iter()
        .enumerate()
        .map(|(i, (label, shortcut))| {
            let selected = is_focused && i == app.sidebar_index;
            let prefix = if selected { "❯ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let shortcut_style = Style::default().fg(Color::DarkGray);

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(*label, style),
                Span::styled(format!(" [{shortcut}]"), shortcut_style),
            ]))
        })
        .collect();

    let title = Line::from(vec![Span::styled(
        "COMMANDS",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )]);

    let list = List::new(items).block(
        Block::default()
            .title(title)
            .borders(Borders::NONE),
    );

    f.render_widget(list, area);
}

fn draw_worktrees(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.active_panel == ActivePanel::Sidebar
        && app.active_action == ActiveAction::None
        && app.sidebar_index >= COMMANDS.len();

    let items: Vec<ListItem> = if app.worktrees_loading {
        vec![ListItem::new(Span::styled(
            "  Loading...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else if let Some(ref err) = app.worktrees_error {
        vec![ListItem::new(Span::styled(
            format!("  ✗ {err}"),
            Style::default().fg(Color::Red),
        ))]
    } else if app.worktrees.is_empty() {
        vec![ListItem::new(Span::styled(
            "  No worktrees found",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.worktrees
            .iter()
            .enumerate()
            .map(|(i, wt)| {
                let list_index = COMMANDS.len() + i;
                let selected = is_focused && list_index == app.sidebar_index;

                let prefix = if selected { "❯ " } else { "  " };
                let marker = if wt.is_current { "✦ " } else { "  " };

                let branch_color = if wt.is_current {
                    Color::Green
                } else {
                    Color::White
                };

                let base_style = if selected {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(branch_color)
                };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, base_style),
                    Span::styled(marker, Style::default().fg(Color::Green)),
                    Span::styled(wt.branch.clone(), base_style),
                    Span::styled(
                        format!(" {}", &wt.sha[..wt.sha.len().min(7)]),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect()
    };

    let title = Line::from(vec![Span::styled(
        "WORKTREES",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )]);

    let mut state = ListState::default();
    if is_focused && !app.worktrees.is_empty() {
        let wt_idx = app.sidebar_index.saturating_sub(COMMANDS.len());
        state.select(Some(wt_idx));
    }

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::NONE));

    f.render_stateful_widget(list, area, &mut state);
}

fn draw_sidebar_help(f: &mut Frame, area: Rect) {
    let help = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::DarkGray)),
        Span::styled(" nav  ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
        Span::styled("Tab", Style::default().fg(Color::DarkGray)),
        Span::styled(" panel  ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
        Span::styled("q", Style::default().fg(Color::DarkGray)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
    ]);

    f.render_widget(Paragraph::new(help), area);
}

// ─────────────────────────────────── Console ────────────────────────────────

fn draw_console(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.active_panel == ActivePanel::Console;
    let border_color = if is_focused { Color::Cyan } else { Color::DarkGray };

    let block = Block::default()
        .title(" Console — output ")
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let inner_height = inner.height as usize;
    let lines: Vec<Line> = app
        .messages
        .iter()
        .map(|msg| {
            let color = match msg.kind {
                MessageKind::Command => Color::Cyan,
                MessageKind::Success => Color::Green,
                MessageKind::Error => Color::Red,
                MessageKind::Info => Color::Gray,
            };
            Line::from(Span::styled(msg.text.clone(), Style::default().fg(color)))
        })
        .collect();

    // Tail scroll: show last N lines
    let start = lines.len().saturating_sub(inner_height);
    let visible: Vec<Line> = lines.into_iter().skip(start).collect();

    let paragraph = Paragraph::new(visible).wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

// ──────────────────────────────── Overlays ──────────────────────────────────

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let popup_x = r.x + (r.width.saturating_sub(popup_width)) / 2;
    let popup_y = r.y + (r.height.saturating_sub(height)) / 2;
    Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: height.min(r.height),
    }
}

fn draw_new_branch_overlay(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(60, 7, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" New Branch / Worktree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Branch name: ", Style::default().fg(Color::Gray)),
            Span::styled(&app.input_buffer, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ])),
        layout[0],
    );

    f.render_widget(
        Paragraph::new(Span::styled(
            "Enter to create  Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
        layout[2],
    );
}

fn draw_sync_pr_overlay(f: &mut Frame, app: &App, area: Rect) {
    let height = (app.prs.len() as u16 + 6).min(area.height.saturating_sub(4));
    let popup = centered_rect(70, height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Sync GitHub PR as Worktree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(popup).inner(Margin { horizontal: 1, vertical: 1 });
    f.render_widget(block, popup);

    if app.prs_loading {
        f.render_widget(
            Paragraph::new(Span::styled("Fetching open PRs...", Style::default().fg(Color::DarkGray))),
            inner,
        );
        return;
    }

    if let Some(ref err) = app.prs_error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), Style::default().fg(Color::Red))).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    if app.prs.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("No open pull requests found.", Style::default().fg(Color::DarkGray))),
            inner,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .prs
        .iter()
        .enumerate()
        .map(|(i, pr)| {
            let selected = i == app.overlay_index;
            let prefix = if selected { "❯ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("#{} ", pr.number), Style::default().fg(Color::DarkGray)),
                Span::styled(pr.title.clone(), style),
                Span::styled(format!(" ({})", pr.head_ref_name), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.overlay_index));

    let list_height = inner.height.saturating_sub(1);
    let list_area = Rect { height: list_height, ..inner };
    let help_area = Rect {
        y: inner.y + list_height,
        height: 1,
        ..inner
    };

    f.render_stateful_widget(List::new(items), list_area, &mut state);
    f.render_widget(
        Paragraph::new(Span::styled(
            "↑↓ navigate  Enter checkout  Esc cancel",
            Style::default().fg(Color::DarkGray),
        )),
        help_area,
    );
}

fn draw_delete_overlay(f: &mut Frame, app: &App, area: Rect) {
    let deletable = app.deletable_worktrees();
    let height = (deletable.len() as u16 + 7).min(area.height.saturating_sub(4));
    let popup = centered_rect(60, height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Delete Worktree ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let inner = block.inner(popup).inner(Margin { horizontal: 1, vertical: 1 });
    f.render_widget(block, popup);

    if deletable.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No deletable worktrees.\n(Cannot delete main or current.)",
                Style::default().fg(Color::DarkGray),
            )).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    if app.delete_confirming {
        if let Some(wt) = deletable.get(app.overlay_index) {
            let text = vec![
                Line::from(Span::styled("Delete worktree for branch:", Style::default().fg(Color::Yellow))),
                Line::from(Span::styled(wt.branch.clone(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
                Line::from(Span::styled(wt.path.clone(), Style::default().fg(Color::DarkGray))),
                Line::from(vec![]),
                Line::from(Span::styled("Confirm? [y/n]", Style::default().fg(Color::Yellow))),
            ];
            f.render_widget(Paragraph::new(text), inner);
        }
        return;
    }

    let items: Vec<ListItem> = deletable
        .iter()
        .enumerate()
        .map(|(i, wt)| {
            let selected = i == app.overlay_index;
            let prefix = if selected { "❯ " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(wt.branch.clone(), style),
                Span::styled(
                    format!(" {}", &wt.sha[..wt.sha.len().min(7)]),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.overlay_index));

    let list_height = inner.height.saturating_sub(1);
    let list_area = Rect { height: list_height, ..inner };
    let help_area = Rect {
        y: inner.y + list_height,
        height: 1,
        ..inner
    };

    f.render_stateful_widget(List::new(items), list_area, &mut state);
    f.render_widget(
        Paragraph::new(Span::styled(
            "↑↓ navigate  Enter select  Esc cancel",
            Style::default().fg(Color::DarkGray),
        )),
        help_area,
    );
}
