//! Rendering: state to frame.
//!
//! Deliberately thin. It contains no decisions worth testing: which screen, which
//! rows, which labels, and which layout mode all come from [`crate::state`].
//! Every area is checked before use and every string is clipped by ratatui, so a
//! tiny or zero-sized terminal degrades instead of panicking.
//!
//! Colour, glyphs, and weights come from [`crate::theme`] and nowhere else, so a
//! palette change is one file rather than a hunt through this one.

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::model::MessageView;
use crate::state::{
    Connection, Focus, FriendsFocus, LayoutMode, LoginMode, Notice, RowView, Screen, State,
};
use crate::theme;

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

/// A panel edge: rounded when it has focus, dim when it does not.
fn panel(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if focused {
            theme::focused_border()
        } else {
            theme::border()
        })
        .title(Span::styled(
            format!(" {title} "),
            if focused {
                theme::accent_bold()
            } else {
                theme::muted()
            },
        ))
}

/// Put `right` against the panel's right edge, padding the gap by display width
/// so CJK text on the left cannot push the time out of place.
fn right_aligned(line: Line<'static>, right: &str, width: u16) -> Line<'static> {
    right_aligned_spans(
        line,
        vec![Span::styled(right.to_string(), theme::muted())],
        width,
    )
}

/// Same, for a styled right-hand side (an unread count followed by a time).
fn right_aligned_spans(
    mut line: Line<'static>,
    right: Vec<Span<'static>>,
    width: u16,
) -> Line<'static> {
    if width == 0 || right.is_empty() {
        return line;
    }
    let used = line.width();
    let right_width: usize = right.iter().map(|span| span.width()).sum();
    let gap = (width as usize).saturating_sub(used + right_width);
    line.spans.push(Span::raw(" ".repeat(gap)));
    line.spans.extend(right);
    line
}

fn draw_minimal(frame: &mut Frame, area: Rect, state: &State) {
    let status = state
        .status_line()
        .unwrap_or_else(|| status_text(state).to_string());
    let lines = vec![
        Line::from(Span::styled("窗口太小", theme::title())),
        Line::from(""),
        Line::from(Span::styled(
            format!("{}×{}，请放大终端", area.width, area.height),
            theme::muted(),
        )),
        Line::from(""),
        Line::from(Span::styled(status, theme::muted())),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_login(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let mode = match state.form.mode {
        LoginMode::Login => "登录",
        LoginMode::Register => "注册",
    };

    // A centred card reads as a dialog rather than a page of fields.
    let card_width = area.width.min(52);
    let card_height = area.height.min(10);
    let card = Rect {
        x: area.x + (area.width - card_width) / 2,
        y: area.y + (area.height.saturating_sub(card_height)) / 2,
        width: card_width,
        height: card_height,
    };

    let block = panel(&format!("chat-tui · {mode}"), true);
    let inner = block.inner(card);
    frame.render_widget(block, card);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let inner = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };
    if inner.width == 0 {
        return;
    }

    let mut lines = vec![
        Line::from(""),
        field_line("用户名", &state.form.name, state.form.focus == Focus::Name),
        field_line(
            "密　码",
            &"*".repeat(state.form.password.chars().count()),
            state.form.focus == Focus::Password,
        ),
        Line::from(""),
    ];
    if state.form.busy {
        lines.push(Line::from(Span::styled("正在提交…", theme::accent())));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Tab 切换 · Ctrl-R 切换模式 · Enter 提交",
        theme::muted(),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let (marker, style) = if focused {
        (theme::PROMPT, theme::accent())
    } else {
        (" ", Style::default())
    };
    let mut spans = vec![
        Span::styled(format!("{marker} "), style),
        Span::styled(
            format!("{label} "),
            if focused {
                theme::accent_bold()
            } else {
                theme::muted()
            },
        ),
        Span::styled(value.to_string(), style),
    ];
    if focused {
        spans.push(Span::styled(theme::CURSOR, style));
    }
    Line::from(spans)
}

fn draw_conversations(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);
    let header = chunks[0];
    let body = chunks[1];
    let input = chunks[2];

    draw_conversations_header(frame, header, state);
    draw_conversations_message_input(frame, input, state);
    if body.height == 0 || body.width == 0 {
        return;
    }
    let (list_area, messages_area) = match state.layout_mode() {
        // Too little width for two columns: stack the panes instead.
        LayoutMode::Compact => {
            let rows = Layout::vertical([
                Constraint::Percentage(45),
                Constraint::Length(1),
                Constraint::Percentage(55),
            ])
            .split(body);
            (rows[0], rows[2])
        }
        _ => {
            let columns = Layout::horizontal([
                Constraint::Percentage(34),
                Constraint::Length(1),
                Constraint::Percentage(66),
            ])
            .split(body);
            (columns[0], columns[2])
        }
    };
    draw_conversation_list(frame, list_area, state);
    if state.open.choosing() {
        draw_open_picker(frame, messages_area, state);
    } else {
        draw_message_pane(frame, messages_area, state);
    }
}

fn draw_conversations_header(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let who = state.user_name().unwrap_or("?");
    let hints = if area.width >= 100 {
        "↑↓ 选择 · Enter 发送 · /open 名字 · Ctrl-F 好友 · Esc 退出"
    } else if area.width >= 60 {
        "↑↓ 选择 · Enter 发送 · /open · Esc 退出"
    } else {
        "↑↓ · Enter · Esc"
    };
    let line = right_aligned(
        Line::from(vec![
            Span::styled("chat-tui", theme::accent_bold()),
            Span::raw(" "),
            Span::styled(who.to_string(), theme::muted()),
        ]),
        hints,
        area.width,
    );
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_conversations_message_input(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let mut spans = vec![
        Span::styled(format!("{} ", theme::PROMPT), theme::accent()),
        Span::raw(state.input.clone()),
    ];
    if state.input.is_empty() {
        spans.push(Span::styled("输入消息，Enter 发送", theme::muted()));
    } else {
        spans.push(Span::styled(theme::CURSOR, theme::accent()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_conversation_list(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // One accent edge per screen: the conversation list is what the keyboard
    // drives, so it carries the accent and the message pane stays quiet.
    let block = panel("会话", true);
    let list_area = block.inner(area);
    frame.render_widget(block, area);
    if list_area.height == 0 || list_area.width == 0 {
        return;
    }
    if state.conversations.is_empty() {
        let hint = if state.loading {
            "加载会话中…"
        } else {
            "还没有会话\n\n用 /open 用户名 开始一段对话"
        };
        frame.render_widget(
            Paragraph::new(hint)
                .style(theme::muted())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            list_area,
        );
        return;
    }

    let now = Local::now().naive_local();
    // Two lines per conversation reads better than one crowded line, but only
    // when there is width to spare.
    let two_line = list_area.width >= 24 && list_area.height >= 6;
    let show_time = list_area.width >= 22;
    let items: Vec<ListItem> = state
        .conversations
        .iter()
        .map(|conversation| {
            ListItem::new(row_lines(
                conversation.row_view(now),
                two_line,
                show_time,
                list_area.width,
            ))
        })
        .collect();
    render_selectable_list(frame, list_area, items, state.highlighted_index());
}

fn row_lines(row: RowView, two_line: bool, show_time: bool, width: u16) -> Vec<Line<'static>> {
    // Three unread states, one glyph column: filled count, hollow never-read,
    // and a dim tick for read. Nothing else competes for the gutter.
    let (glyph, dot_style) = match row.unread_badge.as_deref() {
        Some("●") => (theme::DOT_UNREAD_ALL, theme::accent()),
        Some(_) => (theme::DOT_UNREAD, theme::accent()),
        None => (theme::DOT_READ, theme::muted()),
    };
    let head = Line::from(vec![
        Span::styled(glyph, dot_style),
        Span::raw(" "),
        Span::styled(row.title, theme::title()),
    ]);
    let mut tail = Vec::new();
    // The hollow glyph already says "never read", so only a real count goes on
    // the right.
    if let Some(badge) = row.unread_badge.clone().filter(|badge| badge != "●") {
        tail.push(Span::styled(badge, theme::accent_bold()));
        tail.push(Span::raw("  "));
    }
    if show_time {
        tail.push(Span::styled(row.time_label.clone(), theme::muted()));
    }
    let head = right_aligned_spans(head, tail, width);
    if !two_line {
        return vec![head];
    }
    vec![
        head,
        Line::from(Span::styled(format!("  {}", row.preview), theme::muted())),
    ]
}

/// Several `/open` matches: the user picks one. It takes the message pane's
/// space, keeping the conversation list visible so cancelling returns exactly
/// where the user was.
fn draw_open_picker(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let block = panel(&format!("选择用户 · /open {}", state.open.query), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑↓ 选择 · Enter 打开 · Esc 取消",
            theme::muted(),
        ))),
        chunks[0],
    );
    if state.open.choices.is_empty() {
        return;
    }
    let items: Vec<ListItem> = state
        .open
        .choices
        .iter()
        .map(|user| ListItem::new(format!("{}  #{}", user.name, user.id)))
        .collect();
    render_selectable_list(frame, chunks[1], items, Some(state.open.selected));
}

fn draw_message_pane(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let title = match state.active_title() {
        Some(title) => title,
        None => "消息".to_string(),
    };
    let block = panel(&title, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let now = Local::now().naive_local();
    let views = state.message_views(now);
    if views.is_empty() {
        let hint = if state.messages_loading {
            "加载聊天记录中…"
        } else if state.active.is_some() {
            // An empty conversation opened with /open is ready for its first
            // message; say so instead of looking broken.
            "还没有消息\n\n在下面输入，Enter 发送"
        } else {
            "从左侧选择一位对话"
        };
        frame.render_widget(
            Paragraph::new(hint)
                .style(theme::muted())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }

    let mut lines = Vec::with_capacity(views.len() * 2);
    let mut previous: Option<(bool, String)> = None;
    for view in views {
        let sender_key = (view.mine, view.sender.clone());
        let grouped = previous.as_ref() == Some(&sender_key);
        // A blank line separates speakers; a burst from one speaker reads as a
        // block with no repeated name.
        if previous.is_some() && !grouped {
            lines.push(Line::from(""));
        }
        lines.extend(message_lines(view, grouped, inner.width));
        previous = Some(sender_key);
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

/// One message: a header naming the sender with a right-aligned time, then the
/// body indented under it. Consecutive messages from the same sender drop the
/// header, so a burst of messages reads as a block instead of a stack of
/// repeated names.
fn message_lines(view: MessageView, grouped: bool, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !grouped {
        let who = if view.mine {
            "我".to_string()
        } else {
            view.sender
        };
        let who_style = if view.mine {
            theme::accent_bold()
        } else {
            theme::title()
        };
        let head = Line::from(vec![Span::styled(who, who_style)]);
        let head = right_aligned(head, &view.time_label, width);
        lines.push(head);
    }
    let body_style = if view.failed {
        Style::default().fg(theme::danger())
    } else {
        Style::default()
    };
    for row in view.body.split('\n') {
        lines.push(Line::from(Span::styled(format!("  {row}"), body_style)));
    }
    if view.failed {
        lines.push(Line::from(Span::styled("  · 发送失败", theme::muted())));
    }
    lines
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
    let hints = "Tab 切换 · Enter 搜索/发送/通过 · r 拒绝 · Esc 返回";
    frame.render_widget(
        Paragraph::new(right_aligned(
            Line::from(vec![
                Span::styled("好友", theme::accent_bold()),
                Span::raw(" "),
                Span::styled(who.to_string(), theme::muted()),
            ]),
            hints,
            area.width,
        )),
        chunks[0],
    );

    let search_focused = state.friends.focus == FriendsFocus::Search;
    let search_style = if search_focused {
        theme::accent()
    } else {
        Style::default()
    };
    let mut search = vec![
        Span::styled(
            if search_focused { theme::PROMPT } else { " " },
            search_style,
        ),
        Span::raw(" "),
        Span::styled(
            if state.friends.query.is_empty() {
                "搜索用户名".to_string()
            } else {
                state.friends.query.clone()
            },
            if state.friends.query.is_empty() || !search_focused {
                theme::muted()
            } else {
                search_style
            },
        ),
    ];
    if search_focused {
        search.push(Span::styled(theme::CURSOR, search_style));
    }
    frame.render_widget(Paragraph::new(Line::from(search)), chunks[1]);

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
        "待处理请求",
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
        "还没有好友\n\n在上面搜索并发送请求"
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
    let block = panel(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(empty_hint)
                .style(theme::muted())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    render_selectable_list(frame, inner, items, Some(selected));
}

/// The one selectable-list body: reversed highlight, blank symbol, clamped
/// selection. Conversations, `/open` matches, and friends lists all use it so
/// the highlight cannot drift between them.
fn render_selectable_list(
    frame: &mut Frame,
    area: Rect,
    items: Vec<ListItem>,
    selected: Option<usize>,
) {
    if area.height == 0 || area.width == 0 || items.is_empty() {
        return;
    }
    let len = items.len();
    let list = List::new(items)
        .highlight_style(theme::selected())
        .highlight_symbol("");
    let mut list_state = ListState::default();
    list_state.select(selected.map(|index| index.min(len - 1)));
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_status(frame: &mut Frame, area: Rect, state: &State) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    if state.screen == Screen::Friends {
        let (text, colour) = friends_status(state);
        render_status(frame, area, colour, &text);
        return;
    }
    let (text, colour) = match state.status_line() {
        // A down push link is a warning, not a failed request; the ordinary
        // status text, if any, is appended to it by the state machine.
        Some(message) if state.connection == Connection::Down => (message, theme::warning()),
        Some(message) => (message, theme::danger()),
        None if state.loading => ("加载中…".to_string(), theme::warning()),
        None => (status_text(state).to_string(), theme::ACCENT),
    };
    render_status(frame, area, colour, &text);
}

/// A state dot plus its text: the dot carries the colour, the sentence carries
/// the meaning, so the state never depends on colour alone.
fn render_status(frame: &mut Frame, area: Rect, colour: Color, text: &str) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} ", theme::DOT_UNREAD),
                Style::default().fg(colour),
            ),
            Span::styled(text.to_string(), theme::muted()),
        ])),
        area,
    );
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
fn friends_status(state: &State) -> (String, Color) {
    match &state.friends.notice {
        Some(Notice::Info(message)) => (message.clone(), theme::success()),
        Some(Notice::Error(message)) => (message.clone(), theme::danger()),
        None if state.friends.loading_requests || state.friends.loading_friends => {
            ("正在加载好友数据…".to_string(), theme::warning())
        }
        None => (
            "搜索用户，或处理待处理的好友请求".to_string(),
            theme::ACCENT,
        ),
    }
}
