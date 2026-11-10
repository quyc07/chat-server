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

use crate::state::{Focus, FriendsFocus, LayoutMode, LoginMode, Notice, RowView, Screen, State};

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
        Screen::Friends => draw_friends(frame, body, state),
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
    let list_area = chunks[1];

    let who = state.user_name().unwrap_or("?");
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("会话", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(who.to_string(), Style::default().fg(Color::DarkGray)),
            Span::raw("   "),
            Span::styled(
                "↑/↓ 选择 · 到底部加载更多 · Ctrl-F 好友 · q 退出",
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        header,
    );

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

fn draw_friends(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Min(3),
        Constraint::Min(3),
    ])
    .split(area);

    let who = state.user_name().unwrap_or("?");
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("好友", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(who.to_string(), Style::default().fg(Color::DarkGray)),
            Span::raw("   "),
            Span::styled(
                "Tab 切换 · Enter 搜索/发送/通过 · r 拒绝 · Esc 返回",
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        chunks[0],
    );

    let search_focused = state.friends.focus == FriendsFocus::Search;
    let search_style = if search_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(if search_focused { "❯" } else { " " }, search_style),
            Span::raw(" 搜索 "),
            Span::styled(
                state.friends.query.clone(),
                search_style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(if search_focused { "▏" } else { "" }, search_style),
        ])),
        chunks[1],
    );

    let now = Local::now().naive_local();
    let results: Vec<ListItem> = state
        .friends
        .results
        .iter()
        .map(|user| ListItem::new(format!("{}  #{}", user.name, user.id)))
        .collect();
    let result_hint = if state.friends.searching {
        "搜索中…"
    } else {
        "输入用户名后按 Enter 搜索"
    };
    draw_friend_list(
        frame,
        chunks[2],
        "搜索结果",
        state.friends.focus == FriendsFocus::Results,
        results,
        state.friends.selected_result,
        result_hint,
    );

    let requests: Vec<ListItem> = state
        .friends
        .requests
        .iter()
        .map(|request| {
            let reason = request
                .reason
                .as_deref()
                .filter(|reason| !reason.is_empty())
                .map(|reason| format!(" · {reason}"))
                .unwrap_or_default();
            let time = crate::model::time_label(request.create_time, now);
            ListItem::new(format!("{}  {}{}", request.request_name, time, reason))
        })
        .collect();
    let request_hint = if state.friends.loading_requests {
        "加载中…"
    } else {
        "没有待处理的好友请求"
    };
    draw_friend_list(
        frame,
        chunks[3],
        "待处理请求 · Enter 通过 · r 拒绝",
        state.friends.focus == FriendsFocus::Requests,
        requests,
        state.friends.selected_request,
        request_hint,
    );

    let friends: Vec<ListItem> = state
        .friends
        .friends
        .iter()
        .map(|friend| ListItem::new(friend.name.clone()))
        .collect();
    let friend_hint = if state.friends.loading_friends {
        "加载中…"
    } else {
        "还没有好友，先搜索并发送请求"
    };
    draw_friend_list(
        frame,
        chunks[4],
        "好友",
        state.friends.focus == FriendsFocus::Friends,
        friends,
        state.friends.selected_friend,
        friend_hint,
    );
}

fn draw_friend_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    focused: bool,
    items: Vec<ListItem>,
    selected: usize,
    empty_hint: &str,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" {title} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(empty_hint).style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }
    let len = items.len();
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("");
    let mut list_state = ListState::default();
    list_state.select(Some(selected.min(len - 1)));
    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn draw_status(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    if state.screen == Screen::Friends {
        let (text, style) = friends_status(state);
        frame.render_widget(Paragraph::new(text).style(style), area);
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
        Screen::Friends => "好友",
    }
}

/// Info notices (already a friend, request already waiting, forbidden) are not
/// failures, so they must not borrow the error colour.
fn friends_status(state: &State) -> (String, Style) {
    match &state.friends.notice {
        Some(Notice::Info(message)) => (message.clone(), Style::default().fg(Color::Green)),
        Some(Notice::Error(message)) => (message.clone(), Style::default().fg(Color::Red)),
        None if state.friends.loading_requests || state.friends.loading_friends => (
            "正在加载好友数据…".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        None => (
            "搜索用户，或处理待处理的好友请求".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    }
}
