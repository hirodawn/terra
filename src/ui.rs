//! Rendering: layout, editor pane, and preview pane.

use crate::app::{rows_for_line, segments, App, Focus};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let bg = app.theme.bg();
    let fg = app.theme.fg();

    // clear background
    f.render_widget(
        Block::default().style(Style::default().bg(bg).fg(fg)),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    draw_title_bar(f, app, chunks[0]);
    draw_main(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);
    draw_selection(f, app);

    if app.show_help {
        draw_help(f, area);
    }
    if app.outline_open {
        draw_outline(f, app, area);
    }
    if app.in_command {
        // cursor in command line is the status bar; handled separately
    }
}

fn draw_selection(f: &mut Frame, app: &App) {
    if let (Some((sx, sy)), Some((ex, ey))) = (app.sel_start, app.sel_end) {
        let (lo_x, hi_x) = if sx <= ex { (sx, ex) } else { (ex, sx) };
        let (lo_y, hi_y) = if sy <= ey { (sy, ey) } else { (ey, sy) };
        for y in lo_y..=hi_y {
            for x in lo_x..=hi_x {
                if x < f.area().width && y < f.area().height {
                    let cell = &mut f.buffer_mut()[(x, y)];
                    let s = cell.style();
                    let fg = s.fg.unwrap_or(Color::Reset);
                    let bg = s.bg.unwrap_or(Color::Reset);
                    cell.set_style(Style::default().fg(bg).bg(fg));
                }
            }
        }
    }
}

fn draw_title_bar(f: &mut Frame, app: &App, area: Rect) {
    let accent = app.theme.accent();
    let title = format!(" ✦ {} ", app.title);
    let dirty = if app.buf.dirty { " ● unsaved " } else { "" };
    let right = format!(
        " {} │ {} │ wrap:{} │ {} ",
        app.theme.name(),
        app.preview_theme,
        if app.wrap { "on" } else { "off" },
        if app.focus == Focus::Editor { "EDIT" } else { "VIEW" }
    );
    let line = Line::from(vec![
        Span::styled(title, Style::default().fg(Color::Black).bg(accent).add_modifier(Modifier::BOLD)),
        Span::styled(dirty, Style::default().fg(Color::LightRed)),
        Span::raw(" "),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ]);
    let p = Paragraph::new(line).style(Style::default().bg(Color::Reset));
    f.render_widget(p, area);
}

fn draw_main(f: &mut Frame, app: &mut App, area: Rect) {
    let half = area.width / 2;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(half), Constraint::Min(1)])
        .split(area);

    // record sizes for app logic
    app.editor_width = cols[0].width as usize;
    app.editor_height = cols[0].height as usize;
    app.preview_width = cols[1].width as usize;
    app.preview_height = cols[1].height as usize;

    draw_editor(f, app, cols[0]);
    draw_preview(f, app, cols[1]);
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Editor;
    let accent = app.theme.accent();
    let border = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(if focused { accent } else { Color::DarkGray }))
        .title(Span::styled(
            " editor ",
            Style::default().fg(if focused { Color::Black } else { Color::DarkGray }).bg(if focused { accent } else { Color::Reset }),
        ));
    let inner = border.inner(area);
    app.ei_x = inner.x;
    app.ei_y = inner.y;
    app.ei_w = inner.width;
    f.render_widget(border, area);

    let gutter = app.gutter_width();
    let avail = inner.width as usize;
    let text_w = if app.wrap {
        avail.saturating_sub(gutter + 1).max(1)
    } else {
        usize::MAX / 2
    };

    // gather visible rows
    let mut rows: Vec<(usize, usize, usize)> = Vec::new(); // (line_idx, seg_idx, x_off)
    let mut line_start_row: Vec<usize> = Vec::with_capacity(app.buf.line_count());
    let mut cur_row = 0usize;
    for i in 0..app.buf.line_count() {
        line_start_row.push(cur_row);
        let chars = app.buf.line_chars(i);
        let n = if app.wrap { rows_for_line(&chars, text_w) } else { 1 };
        for s in 0..n {
            rows.push((i, s, 0));
        }
        cur_row += n;
    }

    // build display lines
    let scroll = app.editor_scroll;
    let height = inner.height as usize;
    let visible = rows.iter().skip(scroll).take(height).cloned().collect::<Vec<_>>();

    let mut y = inner.y;
    let mut cursor_xy: Option<(u16, u16)> = None;
    let (crow, _cx) = app.editor_cursor_display_row();
    for (line_idx, seg_idx, _) in &visible {
        let chars = app.buf.line_chars(*line_idx);
        let segs = segments(&chars, text_w);
        let seg = segs.get(*seg_idx).copied().unwrap_or((0, 0));
        // gutter
        let gnum = if *seg_idx == 0 {
            format!("{:>width$} ", line_idx + 1, width = gutter - 1)
        } else {
            " ".repeat(gutter)
        };
        let gspan = Span::styled(
            gnum,
            Style::default().fg(if *seg_idx == 0 { Color::DarkGray } else { Color::Reset }),
        );
        // text (with search-match highlighting)
        let text_str: String = chars[seg.0..(seg.0 + seg.1).min(chars.len())].iter().collect();
        let highlight = app.buf.cursor_line == *line_idx;
        let base = Style::default().fg(app.theme.fg()).bg(if highlight && focused { Color::Rgb(32, 36, 48) } else { app.theme.bg() });
        let match_style = Style::default().fg(Color::Black).bg(Color::Yellow);
        let mut spans = vec![gspan];
        if app.last_query.is_empty() {
            spans.push(Span::styled(text_str, base));
        } else {
            let q = app.last_query.to_lowercase();
            let lower = text_str.to_lowercase();
            let mut cursor = 0usize;
            while let Some(rel) = lower[cursor..].find(&q) {
                let abs = cursor + rel;
                if abs > cursor {
                    spans.push(Span::styled(text_str[cursor..abs].to_string(), base));
                }
                spans.push(Span::styled(text_str[abs..abs + q.len()].to_string(), match_style));
                cursor = abs + q.len();
                if q.is_empty() { break; }
            }
            if cursor < text_str.len() {
                spans.push(Span::styled(text_str[cursor..].to_string(), base));
            }
        }
        let line = Line::from(spans);
        f.render_widget(Paragraph::new(line), Rect { x: inner.x, y, width: inner.width, height: 1 });

        // cursor
        if *line_idx == app.buf.cursor_line && focused {
            // determine which seg holds the cursor
            let within = crow - line_start_row[*line_idx];
            if within == *seg_idx {
                let mut x = 0u16;
                for &c in &chars[seg.0..app.buf.cursor_col.min(chars.len()).max(seg.0)] {
                    x += char_cell(c) as u16;
                }
                cursor_xy = Some((inner.x + gutter as u16 + x, y));
            }
        }
        y += 1;
    }

    if !focused {
        // still keep cursor pos but don't show; ratatui hides cursor automatically
    }
    if let Some((x, y)) = cursor_xy {
        f.set_cursor_position((x, y));
    }
}

fn draw_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Preview;
    let accent = app.theme.accent();
    let border = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(if focused { accent } else { Color::DarkGray }))
        .title(Span::styled(
            " preview ",
            Style::default().fg(if focused { Color::Black } else { Color::DarkGray }).bg(if focused { accent } else { Color::Reset }),
        ));
    let inner = border.inner(area);
    app.pi_x = inner.x;
    app.pi_y = inner.y;
    app.pi_w = inner.width;
    f.render_widget(border, area);

    // CSS-grade renderer paints directly into the buffer.
    app.ensure_preview();
    let dark = !matches!(app.theme, crate::app::Theme::Light);
    let h = crate::pretty::render(f, inner, &app.buf.text(), app.preview_scroll, dark, &app.preview_theme);
    app.preview_content_height = h;

    let _ = focused;
}


fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    if app.search_open {
        let line = Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(app.search_query.clone()),
            Span::raw("  (Enter=next · Esc=cancel · n/N in editor)"),
        ]);
        f.render_widget(Paragraph::new(line), area);
        let x = area.x + 1 + app.search_query.chars().count() as u16;
        f.set_cursor_position((x, area.y));
        return;
    }
    if app.in_command {
        let line = Line::from(vec![
            Span::styled(":", Style::default().fg(Color::Yellow)),
            Span::raw(app.command.clone()),
        ]);
        f.render_widget(Paragraph::new(line), area);
        let x = area.x + 1 + app.command.chars().count() as u16;
        f.set_cursor_position((x, area.y));
        return;
    }
    let left = format!(
        " {}:{} ",
        app.buf.cursor_line + 1,
        app.buf.cursor_col + 1
    );
    let mode = if app.focus == Focus::Editor { " INSERT " } else { " NORMAL " };
    let mode_style = if app.focus == Focus::Editor {
        Style::default().fg(Color::Black).bg(Color::LightGreen)
    } else {
        Style::default().fg(Color::Black).bg(Color::LightBlue)
    };
    let status_style = if app.status_is_error {
        Style::default().fg(Color::White).bg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let line = Line::from(vec![
        Span::styled(mode, mode_style),
        Span::styled(left, Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {} ", app.status), status_style),
        Span::raw(format!(
            "  {} words · {}L · rev:{}",
            app.buf.text().split_whitespace().count(),
            app.buf.line_count(),
            app.buf.revision
        )),
    ]);
    f.render_widget(Paragraph::new(line).alignment(Alignment::Left), area);
}

fn draw_outline(f: &mut Frame, app: &App, area: Rect) {
    let headings = app.headings();
    let popup = centered(46, 70, area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.theme.accent()))
            .title(Span::styled(
                " outline — j/k · Enter · Esc ",
                Style::default().fg(Color::Black).bg(app.theme.accent()).add_modifier(Modifier::BOLD),
            )),
        popup,
    );
    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };
    let sel = app.outline_sel;
    let lines: Vec<Line> = headings
        .iter()
        .enumerate()
        .map(|(i, (line, level, title))| {
            let indent = "  ".repeat(level.saturating_sub(1));
            let marker = format!("{} {} ", level, "H".repeat(*level));
            let _ = marker;
            let style = if i == sel {
                Style::default().fg(Color::Black).bg(app.theme.accent())
            } else {
                Style::default().fg(match *level {
                    1 => Color::LightBlue,
                    2 => Color::Cyan,
                    3 => Color::Yellow,
                    _ => Color::Gray,
                })
            };
            let _ = line;
            Line::from(vec![
                Span::styled(format!("{}H{} ", indent, level), style),
                Span::styled(title.clone(), style),
            ])
        })
        .collect();
    let para = Paragraph::new(lines).scroll((0, 0));
    f.render_widget(para, inner);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let popup = centered(60, 70, area);
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        popup,
    );
    let help = vec![
        Line::from(Span::styled(
            " terra — keymap ",
            Style::default().fg(Color::Black).bg(Color::LightBlue).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(vec![Span::styled("  Ctrl+S", Style::default().fg(Color::Yellow)), Span::raw("   save")]),
        Line::from(vec![Span::styled("  Ctrl+Q", Style::default().fg(Color::Yellow)), Span::raw("   quit")]),
        Line::from(vec![Span::styled("  Tab", Style::default().fg(Color::Yellow)), Span::raw("       switch edit/preview")]),
        Line::from(vec![Span::styled("  Ctrl+W", Style::default().fg(Color::Yellow)), Span::raw("   toggle wrap")]),
        Line::from(vec![Span::styled("  Ctrl+Y", Style::default().fg(Color::Yellow)), Span::raw("   toggle preview sync")]),
        Line::from(vec![Span::styled("  Ctrl+T", Style::default().fg(Color::Yellow)), Span::raw("   cycle theme")]),
        Line::from(vec![Span::styled("  Ctrl+E", Style::default().fg(Color::Yellow)), Span::raw("   cycle preview theme")]),
        Line::from(vec![Span::styled("  Ctrl+H / ?", Style::default().fg(Color::Yellow)), Span::raw(" help")]),
        Line::from(vec![Span::styled("  Ctrl+O", Style::default().fg(Color::Yellow)), Span::raw("   outline jump")]),
        Line::from(vec![Span::styled("  /  n  N", Style::default().fg(Color::Yellow)), Span::raw("   search / next / prev")]),
        Line::from(vec![Span::styled("  Ctrl+D", Style::default().fg(Color::Yellow)), Span::raw("   duplicate line")]),
        Line::from(vec![Span::styled("  Ctrl+K", Style::default().fg(Color::Yellow)), Span::raw("   delete line")]),
        Line::from(vec![Span::styled("  :w :q :x", Style::default().fg(Color::Yellow)), Span::raw(" command mode")]),
        Line::from(vec![Span::styled("  :42", Style::default().fg(Color::Yellow)), Span::raw("      go to line 42")]),
        Line::from(vec![Span::styled("  Ctrl+←/→", Style::default().fg(Color::Yellow)), Span::raw(" move by word")]),
        Line::from(vec![Span::styled("  PageUp/Down", Style::default().fg(Color::Yellow)), Span::raw(" scroll page")]),
        Line::raw(""),
        Line::from(Span::styled("  press ? or Esc to close", Style::default().fg(Color::DarkGray))),
    ];
    let p = Paragraph::new(help).style(Style::default().fg(Color::Gray));
    f.render_widget(p, popup);
}

fn centered(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup)[1]
}

fn char_cell(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(0).max(0)
}

