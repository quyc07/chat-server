//! Rendering: state to frame.
//!
//! Deliberately thin. It contains no decisions worth testing: which screen, which
//! rows, which labels, and which layout mode all come from [`crate::state`].
//! Every area is checked before use and every string is clipped by ratatui, so a
//! tiny or zero-sized terminal degrades instead of panicking.

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::model::MessageView;
use crate::state::{Focus, LayoutMode, LoginMode, RowView, Screen, State};

pub fn draw(frame: &mut Frame, state: &State) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    if state.layout_mode() == LayoutMode::Minimal {
        draw_minimal(frame, area, state);
        return;
    }

    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let body = chunks[0];
    let status = chunks[1];
    match state.screen {
        Screen::Login => draw_login(frame, body, state),
        Screen::Conversations => draw_conversations(frame, body, state),
    }
    draw_status(frame, status, state);
}

fn draw_minimal(frame: &mut Frame, area: Rect, state: &State) {
    let status = state
        .status
        .as_deref()
        .unwrap_or_else(|| status_text(state));
    let text = format!(
        "窗口太小 ({}x{})，请放大终端 · {status}",
        area.width, area.height
    );
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
}

fn draw_login(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let mode = match state.form.mode {
        LoginMode::Login => "登录",
        LoginMode::Register => "注册",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" chat-tui · {mode} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let viewport_hint = if state.viewport.width < 50 {
        "Tab 切换字段 · Ctrl-R 切换登录/注册 · Enter 提交"
    } else {
        "Tab/↑↓ 切换字段 · Ctrl-R/F2 切换登录/注册 · Enter 提交 · Ctrl-C 退出"
    };
    let mut lines = vec![
        Line::from(""),
        field_line("用户名", &state.form.name, state.form.focus == Focus::Name),
        field_line(
            "密　码",
            &"*".repeat(state.form.password.chars().count()),
            state.form.focus == Focus::Password,
        ),
        Line::from(""),
        Line::from(Span::styled(
            format!("当前模式：{mode}（Ctrl-R 切换）"),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            viewport_hint,
            Style::default().fg(Color::DarkGray),
        )),
    ];
    if state.form.busy {
        lines.push(Line::from(Span::styled(
            "正在提交…",
            Style::default().fg(Color::Yellow),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let marker = if focused { "❯" } else { " " };
    let style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::styled(marker.to_string(), style),
        Span::raw(" "),
        Span::styled(format!("{label} "), style.add_modifier(Modifier::BOLD)),
        Span::styled(value.to_string(), style),
        Span::styled(if focused { "▏" } else { "" }, style),
    ])
}

fn draw_conversations(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let header = chunks[0];
    let body = chunks[1];
    draw_conversations_header(frame, header, state);
    if body.height == 0 || body.width == 0 {
        return;
    }
    let (list_area, messages_area) = match state.layout_mode() {
        // Too little width for two columns: stack the panes instead.
        LayoutMode::Compact => {
            let rows = Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(body);
            (rows[0], rows[1])
        }
        _ => {
            let columns =
                Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
                    .split(body);
            (columns[0], columns[1])
        }
    };
    draw_conversation_list(frame, list_area, state);
    draw_message_pane(frame, messages_area, state);
}

fn draw_conversations_header(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let who = state.user_name().unwrap_or("?");
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("会话", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(who.to_string(), Style::default().fg(Color::DarkGray)),
            Span::raw("   "),
            Span::styled(
                "↑/↓ 选择 · Enter 重新加载 · PgUp/PgDn 滚动消息 · q 退出",
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        area,
    );
}

fn draw_conversation_list(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let block = Block::default().borders(Borders::ALL).title(" 会话 ");
    let list_area = block.inner(area);
    frame.render_widget(block, area);
    if list_area.height == 0 || list_area.width == 0 {
        return;
    }
    if state.conversations.is_empty() {
        let hint = if state.loading {
            "加载会话中…"
        } else {
            "暂无会话"
        };
        frame.render_widget(
            Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
            list_area,
        );
        return;
    }

    let now = Local::now().naive_local();
    let compact = state.layout_mode() == LayoutMode::Compact;
    let items: Vec<ListItem> = state
        .conversations
        .iter()
        .map(|conversation| ListItem::new(row_line(conversation.row_view(now), compact)))
        .collect();
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("");
    let mut list_state = ListState::default();
    list_state.select(Some(state.selected.min(state.conversations.len() - 1)));
    frame.render_stateful_widget(list, list_area, &mut list_state);
}

fn draw_message_pane(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let title = match state.active_conversation() {
        Some(conversation) => format!(" {} ", conversation.title),
        None => " 消息 ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if state.messages.is_empty() {
        let hint = if state.messages_loading {
            "加载聊天记录中…"
        } else if state.active.is_some() {
            "暂无消息"
        } else {
            "从左侧选择一位好友"
        };
        frame.render_widget(
            Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let now = Local::now().naive_local();
    let mut lines = Vec::with_capacity(state.messages.len() * 3);
    for view in state.message_views(now) {
        lines.extend(message_lines(view));
    }
    // The paragraph wraps every line (CJK included) to the pane width. The state
    // machine only says how many lines to scroll up from the newest message;
    // turning that into a top offset is layout, so it happens here.
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total_lines = paragraph.line_count(inner.width);
    let max_top = total_lines.saturating_sub(inner.height as usize);
    let top = max_top.saturating_sub(state.message_scroll_up as usize);
    let top = top.min(u16::MAX as usize) as u16;
    frame.render_widget(paragraph.scroll((top, 0)), inner);
}

/// One message as a header line (who + time) and one or more body lines. The
/// body is handed to the paragraph unwrapped; the paragraph does the width-aware
/// wrapping so no column is ever overflowed.
fn message_lines(view: MessageView) -> Vec<Line<'static>> {
    let (who, who_style, body_style) = if view.mine {
        (
            "我".to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Cyan),
        )
    } else {
        (
            view.sender,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Style::default(),
        )
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{who} "), who_style),
        Span::styled(view.time_label, Style::default().fg(Color::DarkGray)),
    ])];
    for row in view.body.split('\n') {
        lines.push(Line::from(Span::styled(format!("  {row}"), body_style)));
    }
    lines.push(Line::from(""));
    lines
}

fn row_line(row: RowView, compact: bool) -> Line<'static> {
    let has_badge = row.unread_badge.is_some();
    let badge = row.unread_badge.unwrap_or_else(|| " ".to_string());
    let badge_style = if has_badge {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let mut spans = vec![
        Span::styled(format!("{badge:>2} "), badge_style),
        Span::styled(row.title, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(row.preview, Style::default().fg(Color::Gray)),
    ];
    if !compact {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            row.time_label,
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn draw_status(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let (text, style) = match &state.status {
        Some(message) => (message.clone(), Style::default().fg(Color::Red)),
        None if state.loading => ("加载中…".to_string(), Style::default().fg(Color::DarkGray)),
        None => (
            status_text(state).to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    };
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn status_text(state: &State) -> &'static str {
    match state.screen {
        Screen::Login => "请输入用户名和密码",
        Screen::Conversations => "就绪",
    }
}
