use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, COMMANDS},
    types::{ActiveAction, ActivePanel},
};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Update console size for PTY sizing
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(area);

    let console_inner = Block::default().borders(Borders::ALL).inner(chunks[1]);
    // Expose console dimensions for PTY resizing (subtract border)
    let _ = (console_inner.width, console_inner.height);

    draw_sidebar(f, app, chunks[0]);
    draw_console(f, app, chunks[1]);

    match app.active_action {
        ActiveAction::NewBranch => draw_new_branch_overlay(f, app, area),
        ActiveAction::SyncPr => draw_sync_pr_overlay(f, app, area),
        ActiveAction::Delete => draw_delete_overlay(f, app, area),
        ActiveAction::None => {}
    }
}

// ─────────────────────────────────── Sidebar ────────────────────────────────

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let is_focused =
        app.active_panel == ActivePanel::Sidebar && app.active_action == ActiveAction::None;
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
            Constraint::Length(COMMANDS.len() as u16 + 2),
            Constraint::Min(4),
            Constraint::Length(2),
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
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(*label, style),
                Span::styled(
                    format!(" [{shortcut}]"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let title = Line::from(vec![Span::styled(
        "COMMANDS",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )]);

    f.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::NONE)),
        area,
    );
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
                let is_active_shell = app.active_pty_path.as_deref() == Some(&wt.path);

                let prefix = if selected { "❯ " } else { "  " };
                let shell_dot = if is_active_shell { "● " } else if app.pty_sessions.contains_key(&wt.path) { "○ " } else { "  " };

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
                let dot_color = if is_active_shell { Color::Green } else { Color::DarkGray };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, base_style),
                    Span::styled(shell_dot, Style::default().fg(dot_color)),
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
        state.select(Some(app.sidebar_index.saturating_sub(COMMANDS.len())));
    }

    f.render_stateful_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::NONE)),
        area,
        &mut state,
    );
}

fn draw_sidebar_help(f: &mut Frame, area: Rect) {
    let help = if true {
        Line::from(vec![
            Span::styled("↑↓", Style::default().fg(Color::DarkGray)),
            Span::styled(" nav  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::DarkGray)),
            Span::styled(" open shell  ", Style::default().fg(Color::DarkGray)),
            Span::styled("^Space", Style::default().fg(Color::DarkGray)),
            Span::styled(" focus", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::default()
    };
    f.render_widget(Paragraph::new(help), area);
}

// ─────────────────────────────────── Console ────────────────────────────────

pub fn draw_console(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.active_panel == ActivePanel::Console;
    let border_color = if is_focused { Color::Cyan } else { Color::DarkGray };

    let (title_text, title_color) = match &app.active_pty_path {
        Some(path) => {
            let branch = app
                .worktrees
                .iter()
                .find(|wt| wt.path == *path)
                .map(|wt| wt.branch.as_str())
                .unwrap_or(path.as_str());
            (format!(" ⎇  {branch} "), Color::Green)
        }
        None => (" Shell — select a worktree to start ".to_string(), Color::DarkGray),
    };

    let hint = if is_focused {
        " ^Space: sidebar "
    } else {
        " ^Space / Enter: focus "
    };

    let block = Block::default()
        .title(title_text.as_str())
        .title_alignment(Alignment::Left)
        .title_style(Style::default().fg(title_color).add_modifier(Modifier::BOLD))
        .title_bottom(hint)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    match &app.active_pty_path {
        None => {
            let msg = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Select a worktree from the sidebar and press Enter",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "  to open an interactive shell in that directory.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]);
            f.render_widget(msg, inner);
        }
        Some(path) => {
            if let Some(session) = app.pty_sessions.get(path) {
                draw_pty_screen(f, session, inner, is_focused);
            }
        }
    }
}

fn draw_pty_screen(f: &mut Frame, session: &crate::pty::PtySession, area: Rect, show_cursor: bool) {
    let Ok(parser) = session.parser.lock() else { return };
    let screen = parser.screen();
    let rows = area.height as usize;
    let cols = area.width as usize;

    let mut lines: Vec<Line> = Vec::with_capacity(rows);

    for row in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();

        for col in 0..cols {
            let (ch, style) = match screen.cell(row as u16, col as u16) {
                Some(cell) => {
                    let ch_str = cell.contents();
                    let ch = if ch_str.is_empty() { " ".to_string() } else { ch_str.to_string() };

                    let fg = vt100_color_to_ratatui(cell.fgcolor());
                    let bg = vt100_color_to_ratatui(cell.bgcolor());

                    let mut style = Style::default().fg(fg).bg(bg);
                    if cell.bold() {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if cell.italic() {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if cell.underline() {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    if cell.inverse() {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    (ch, style)
                }
                None => (" ".to_string(), Style::default()),
            };

            if style == current_style {
                current_text.push_str(&ch);
            } else {
                if !current_text.is_empty() {
                    spans.push(Span::styled(current_text.clone(), current_style));
                }
                current_text = ch;
                current_style = style;
            }
        }
        if !current_text.is_empty() {
            spans.push(Span::styled(current_text, current_style));
        }

        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), area);

    // Show cursor
    if show_cursor && !screen.hide_cursor() {
        let (cursor_row, cursor_col) = screen.cursor_position();
        let x = area.x + cursor_col;
        let y = area.y + cursor_row;
        if x < area.x + area.width && y < area.y + area.height {
            f.set_cursor_position((x, y));
        }
    }
}

fn vt100_color_to_ratatui(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(0) => Color::Black,
        vt100::Color::Idx(1) => Color::Red,
        vt100::Color::Idx(2) => Color::Green,
        vt100::Color::Idx(3) => Color::Yellow,
        vt100::Color::Idx(4) => Color::Blue,
        vt100::Color::Idx(5) => Color::Magenta,
        vt100::Color::Idx(6) => Color::Cyan,
        vt100::Color::Idx(7) => Color::Gray,
        vt100::Color::Idx(8) => Color::DarkGray,
        vt100::Color::Idx(9) => Color::LightRed,
        vt100::Color::Idx(10) => Color::LightGreen,
        vt100::Color::Idx(11) => Color::LightYellow,
        vt100::Color::Idx(12) => Color::LightBlue,
        vt100::Color::Idx(13) => Color::LightMagenta,
        vt100::Color::Idx(14) => Color::LightCyan,
        vt100::Color::Idx(15) => Color::White,
        vt100::Color::Idx(n) => Color::Indexed(n),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
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
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
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

    let inner = block
        .inner(popup)
        .inner(Margin { horizontal: 1, vertical: 1 });
    f.render_widget(block, popup);

    if app.prs_loading {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Fetching open PRs...",
                Style::default().fg(Color::DarkGray),
            )),
            inner,
        );
        return;
    }

    if let Some(ref err) = app.prs_error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(Color::Red),
            ))
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    if app.prs.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No open pull requests found.",
                Style::default().fg(Color::DarkGray),
            )),
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
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(
                    format!("#{} ", pr.number),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(pr.title.clone(), style),
                Span::styled(
                    format!(" ({})", pr.head_ref_name),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.overlay_index));

    let list_height = inner.height.saturating_sub(1);
    let list_area = Rect { height: list_height, ..inner };
    let help_area = Rect { y: inner.y + list_height, height: 1, ..inner };

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

    let inner = block
        .inner(popup)
        .inner(Margin { horizontal: 1, vertical: 1 });
    f.render_widget(block, popup);

    if deletable.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No deletable worktrees.\n(Cannot delete main or current.)",
                Style::default().fg(Color::DarkGray),
            ))
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    if app.delete_confirming {
        if let Some(wt) = deletable.get(app.overlay_index) {
            let text = vec![
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
                    "Confirm? [y/n]",
                    Style::default().fg(Color::Yellow),
                )),
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
    let help_area = Rect { y: inner.y + list_height, height: 1, ..inner };

    f.render_stateful_widget(List::new(items), list_area, &mut state);
    f.render_widget(
        Paragraph::new(Span::styled(
            "↑↓ navigate  Enter select  Esc cancel",
            Style::default().fg(Color::DarkGray),
        )),
        help_area,
    );
}


