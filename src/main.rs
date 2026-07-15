//! terra — a blazing-fast TUI Markdown editor with live preview.

mod app;
mod buffer;
mod diagram;
mod markdown;
mod pretty;
mod ui;

use app::{App, Focus};
use clap::Parser;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::Terminal;
use std::io::{self, Stdout, Write};
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "terra", version, about = "A blazing-fast TUI Markdown editor with live preview")]
struct Args {
    /// File to open. If omitted, starts with a blank buffer.
    file: Option<String>,
    /// Start in preview focus.
    #[arg(short = 'r', long)]
    read: bool,
    /// Headless: render one frame to stdout and exit (for testing).
    #[arg(long)]
    dump: bool,
    /// Width to use with --dump.
    #[arg(long, default_value = "120")]
    width: u16,
    /// Height to use with --dump.
    #[arg(long, default_value = "40")]
    height: u16,
    /// Goto line before dumping (for testing).
    #[arg(long)]
    goto: Option<usize>,
    /// Preview scroll offset for dump (for testing).
    #[arg(long)]
    pscroll: Option<usize>,
    /// Force light preview theme for dump (for testing).
    #[arg(long)]
    light: bool,
    /// Show outline popup in dump (for testing).
    #[arg(long)]
    outline: bool,
    /// Run a search and jump before dumping (for testing).
    #[arg(long)]
    search: Option<String>,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let (buf, title) = match &args.file {
        Some(path) => match buffer::Buffer::from_path(path) {
            Ok(b) => {
                let t = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string();
                (b, t)
            }
            Err(e) => {
                // create new buffer that will be saved to this path
                let mut b = buffer::Buffer::new("");
                b.path = Some(std::path::PathBuf::from(path));
                b.dirty = true;
                let t = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string();
                eprintln!("note: could not open {path}: {e}; starting empty buffer");
                (b, t)
            }
        },
        None => (buffer::Buffer::new("# Untitled\n\nStart writing...\n"), "untitled.md".to_string()),
    };

    let mut app = App::new(buf, title);
    if args.read {
        app.focus = Focus::Preview;
    }

    if args.dump {
        // set sizes first so cursor-visibility math is sane
        app.editor_width = (args.width / 2) as usize;
        app.editor_height = args.height as usize;
        app.preview_width = (args.width / 2) as usize;
        app.preview_height = args.height as usize;
        if let Some(n) = args.goto {
            app.goto_line(n);
        }
        if let Some(s) = args.pscroll {
            app.preview_scroll = s;
        }
        if args.light {
            app.theme = app::Theme::Light;
            app.preview_theme = "InspiredGitHub".to_string();
        }
        if args.outline {
            app.outline_open = true;
        }
        if let Some(q) = &args.search {
            app.last_query = q.clone();
            app.find_next(q, true);
        }
        return dump_frame(&mut app, args.width, args.height);
    }

    setup_terminal()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run(&mut terminal, &mut app);

    restore_terminal()?;
    result
}

fn setup_terminal() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    Ok(())
}

fn restore_terminal() -> io::Result<()> {
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    let tick = Duration::from_millis(120);
    loop {
        // Ensure cursor visible logic
        if app.focus == Focus::Editor {
            app.ensure_preview();
            app.ensure_cursor_visible();
            app.sync_preview_to_cursor();
        } else {
            app.ensure_preview();
            app.preview_scroll = app.preview_scroll.min(app.preview_content_height.saturating_sub(1));
        }

        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    handle_key(key, app);
                }
                Event::Mouse(m) => handle_mouse(m, app),
                Event::Resize(_, _) => {
                    app.cache_width = 0; // force preview rewrap
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_key(key: KeyEvent, app: &mut App) {
    // Global keys
    if app.in_command {
        handle_command_key(key, app);
        return;
    }
    if app.search_open {
        handle_search_key(key, app);
        return;
    }
    if app.outline_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('o') => {
                app.outline_open = false;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.outline_down();
                return;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.outline_up();
                return;
            }
            KeyCode::Enter => {
                app.outline_jump();
                return;
            }
            _ => return,
        }
    }
    if app.show_help {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Enter => {
                app.show_help = false;
                return;
            }
            _ => return,
        }
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match (key.code, ctrl) {
        (KeyCode::Char('c'), true) => {
            if app.in_command {
                app.in_command = false;
                app.command.clear();
            } else {
                app.should_quit = true;
            }
            return;
        }
        (KeyCode::Char('q'), true) => {
            if app.buf.dirty {
                app.set_error("unsaved changes — Ctrl+Q again to force quit");
                app.buf.dirty = false;
            }
            app.should_quit = true;
            return;
        }
        (KeyCode::Char('s'), true) => {
            save(app);
            return;
        }
        (KeyCode::Char('h'), true) => {
            app.show_help = !app.show_help;
            return;
        }
        (KeyCode::Char('o'), true) => {
            app.outline_open = !app.outline_open;
            app.outline_sel = 0;
            return;
        }
        (KeyCode::Char('?'), _) => {
            app.show_help = !app.show_help;
            return;
        }
        (KeyCode::Char('w'), true) => {
            app.toggle_wrap();
            return;
        }
        (KeyCode::Char('y'), true) => {
            app.sync_preview = !app.sync_preview;
            app.set_status(if app.sync_preview { "preview sync: on" } else { "preview sync: off" });
            return;
        }
        (KeyCode::Char('t'), true) => {
            app.cycle_theme();
            return;
        }
        (KeyCode::Char('e'), true) => {
            cycle_preview_theme(app);
            return;
        }
        (KeyCode::Char('z'), true) => {
            if app.focus == Focus::Editor {
                if key.modifiers.contains(KeyModifiers::SHIFT) { app.buf.redo(); }
                else { app.buf.undo(); }
                app.ensure_cursor_visible();
            }
            return;
        }
        (KeyCode::Char('r'), true) => {
            if app.focus == Focus::Editor { app.buf.redo(); app.ensure_cursor_visible(); }
            return;
        }
        (KeyCode::Tab, _) => {
            app.switch_focus();
            return;
        }
        (KeyCode::Char('d'), true) => {
            if app.focus == Focus::Editor { app.buf.duplicate_line(); app.ensure_cursor_visible(); }
            return;
        }
        (KeyCode::Char('k'), true) => {
            if app.focus == Focus::Editor { app.buf.delete_line(); app.ensure_cursor_visible(); }
            return;
        }
        (KeyCode::Esc, _) => {
            app.last_query.clear();
            app.set_status("ready");
            return;
        }
        (KeyCode::Char(':'), _) => {
            app.in_command = true;
            app.command.clear();
            return;
        }
        (KeyCode::Char('/'), _) => {
            if app.focus == Focus::Editor {
                app.search_open = true;
                app.search_query.clear();
                return;
            }
        }
        (KeyCode::Char('n'), _) => {
            if app.focus == Focus::Editor && !app.last_query.is_empty() {
                app.find_next(&app.last_query.clone(), true);
                return;
            }
        }
        (KeyCode::Char('N'), _) => {
            if app.focus == Focus::Editor && !app.last_query.is_empty() {
                app.find_next(&app.last_query.clone(), false);
                return;
            }
        }
        _ => {}
    }

    if app.focus == Focus::Preview {
        handle_preview_key(key, app);
    } else {
        handle_editor_key(key, app);
    }
}

fn handle_editor_key(key: KeyEvent, app: &mut App) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Left => {
            if ctrl {
                app.buf.word_back();
            } else {
                app.buf.left();
            }
        }
        KeyCode::Right => {
            if ctrl {
                app.buf.word_forward();
            } else {
                app.buf.right();
            }
        }
        KeyCode::Up => app.display_up(),
        KeyCode::Down => app.display_down(),
        KeyCode::Home => app.buf.line_start(),
        KeyCode::End => app.buf.line_end(),
        KeyCode::PageUp => app.page_up(),
        KeyCode::PageDown => app.page_down(),
        KeyCode::Backspace => app.buf.backspace(),
        KeyCode::Delete => app.buf.delete(),
        KeyCode::Enter => app.buf.insert_newline_smart(),
        KeyCode::Char(c) => {
            // ignore control-prefixed chars already handled
            app.buf.insert_char(c);
        }
        _ => {}
    }
}

fn handle_preview_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.preview_scroll = app.preview_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = app.preview_content_height.saturating_sub(app.preview_height);
            if app.preview_scroll < max {
                app.preview_scroll += 1;
            }
        }
        KeyCode::PageUp => {
            app.preview_scroll = app.preview_scroll.saturating_sub(app.preview_height.saturating_sub(1));
        }
        KeyCode::PageDown => {
            let max = app.preview_content_height.saturating_sub(app.preview_height);
            app.preview_scroll = (app.preview_scroll + app.preview_height.saturating_sub(1)).min(max);
        }
        KeyCode::Char('g') => app.preview_scroll = 0,
        KeyCode::Char('G') => {
            app.preview_scroll = app.preview_content_height.saturating_sub(app.preview_height);
        }
        _ => {}
    }
}

fn handle_search_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.search_open = false;
        }
        KeyCode::Enter => {
            app.last_query = app.search_query.clone();
            let q = app.search_query.clone();
            app.search_open = false;
            app.find_next(&q, true);
        }
        KeyCode::Backspace => {
            app.search_query.pop();
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
        }
        _ => {}
    }
}

fn handle_command_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.in_command = false;
            app.command.clear();
        }
        KeyCode::Enter => {
            let cmd = app.command.trim().to_string();
            app.in_command = false;
            app.command.clear();
            run_command(&cmd, app);
        }
        KeyCode::Backspace => {
            app.command.pop();
            if app.command.is_empty() && false {
                app.in_command = false;
            }
        }
        KeyCode::Char(c) => {
            app.command.push(c);
        }
        _ => {}
    }
}

fn run_command(cmd: &str, app: &mut App) {
    let lower = cmd.to_lowercase();
    match lower.as_str() {
        "w" | "write" => save(app),
        "q" | "quit" => app.should_quit = true,
        "x" | "wq" => {
            save(app);
            app.should_quit = true;
        }
        "theme" => app.cycle_theme(),
        s if s.starts_with("theme ") => {
            let _ = &s;
            app.cycle_theme();
        }
        "help" => app.show_help = true,
        s if s.chars().all(|c| c.is_ascii_digit()) => {
            let n: usize = s.parse().unwrap_or(1);
            app.goto_line(n);
        }
        "" => {}
        _ => app.set_error(&format!("unknown command: :{}", cmd)),
    }
}

fn save(app: &mut App) {
    match app.buf.save() {
        Ok(()) => {
            let p = app
                .buf
                .path
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("(no path)")
                .to_string();
            app.set_status(&format!("saved {}", p));
        }
        Err(e) => app.set_error(&format!("save failed: {}", e)),
    }
}

fn cycle_preview_theme(app: &mut App) {
    let themes = [
        "base16-ocean.dark",
        "base16-eighties.dark",
        "base16-mocha.dark",
        "InspiredGitHub",
        "Solarized (dark)",
        "Solarized (light)",
    ];
    let cur = themes.iter().position(|t| *t == app.preview_theme).unwrap_or(0);
    let next = themes[(cur + 1) % themes.len()];
    app.preview_theme = next.to_string();
    app.cached_rev = u64::MAX;
    app.set_status(&format!("preview theme: {}", app.preview_theme));
}

fn handle_mouse(m: MouseEvent, app: &mut App) {
    match m.kind {
        MouseEventKind::ScrollDown => {
            if app.focus == Focus::Preview {
                let max = app.preview_content_height.saturating_sub(app.preview_height);
                if app.preview_scroll < max { app.preview_scroll += 1; }
            } else { app.display_down(); }
        }
        MouseEventKind::ScrollUp => {
            if app.focus == Focus::Preview {
                app.preview_scroll = app.preview_scroll.saturating_sub(1);
            } else { app.display_up(); }
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            // Start text selection within the clicked pane
            let pane = if m.column < app.ei_x + app.ei_w { 0u8 } else { 1u8 };
            app.sel_pane = pane;
            app.sel_start = Some((m.column, m.row));
            app.sel_end = Some((m.column, m.row));
            // also handle click-to-cursor in editor
            if pane == 0 {
                app.focus = Focus::Editor;
                app.click_editor(m.column, m.row);
            } else {
                app.focus = Focus::Preview;
            }
        }
        MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
            // Extend selection, clamped to the pane where it started
            if let Some(_) = app.sel_start {
                let (cx, cy) = clamp_to_pane(app, m.column, m.row);
                app.sel_end = Some((cx, cy));
            }
        }
        MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
            // Copy selected text to clipboard via OSC52
            if let (Some((sx, sy)), Some((ex, ey))) = (app.sel_start, app.sel_end) {
                if (sx, sy) != (ex, ey) {
                    let text = extract_selection_text(app, sx, sy, ex, ey);
                    if !text.is_empty() {
                        osc52_copy(&text);
                        app.set_status("copied to clipboard");
                    }
                }
            }
            app.sel_start = None;
            app.sel_end = None;
        }
        _ => {}
    }
}

fn clamp_to_pane(app: &App, x: u16, y: u16) -> (u16, u16) {
    if app.sel_pane == 0 {
        // editor pane
        let cx = x.min(app.ei_x + app.ei_w.saturating_sub(1)).max(app.ei_x);
        let cy = y;
        (cx, cy)
    } else {
        // preview pane
        let cx = x.min(app.pi_x + app.pi_w.saturating_sub(1)).max(app.pi_x);
        let cy = y;
        (cx, cy)
    }
}

fn extract_selection_text(app: &App, sx: u16, sy: u16, ex: u16, ey: u16) -> String {
    // We need the frame buffer, but don't have it here.
    // Instead, extract from the buffer source text based on the pane.
    // For the editor pane: extract from buf.text() rows.
    // For the preview pane: extract from the rendered preview (approximate from source).
    // Simplest: extract raw text lines from the source between selected rows.
    if app.sel_pane == 0 {
        // Editor: map screen rows to source lines (approximate)
        let text = app.buf.text();
        let lines: Vec<&str> = text.lines().collect();
        let (lo_y, hi_y) = if sy <= ey { (sy, ey) } else { (ey, sy) };
        let scroll = app.editor_scroll;
        let mut result = Vec::new();
        for row in lo_y..=hi_y {
            let line_idx = row as usize + scroll;
            if line_idx < lines.len() {
                result.push(lines[line_idx]);
            }
        }
        result.join("\n")
    } else {
        // Preview: extract from source text (the whole thing for now)
        app.buf.text()
    }
}

fn osc52_copy(text: &str) {
    // OSC52: \x1b]52;c;<base64>\x1b\\
    let b64 = base64_encode(text.as_bytes());
    // Write to stdout (the terminal will intercept OSC52)
    let _ = write!(io::stdout(), "\x1b]52;c;{}\x1b\\", b64);
    let _ = io::stdout().flush();
}

/// Truncate a label to a maximum display width (CJK = 2 cells), appending …

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn dump_frame(app: &mut App, width: u16, height: u16) -> io::Result<()> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    app.ensure_preview();
    app.editor_width = (width / 2) as usize;
    app.editor_height = height as usize;
    app.preview_width = (width / 2) as usize;
    app.preview_height = height as usize;
    terminal.draw(|f| ui::draw(f, app))?;
    print_buffer(terminal.backend(), width, height);
    Ok(())
}

fn print_buffer(backend: &TestBackend, width: u16, height: u16) {
    let buf = backend.buffer();
    let mut out = String::new();
    for y in 0..height {
        let mut line = String::new();
        let mut last_style: Option<ratatui::style::Style> = None;
        for x in 0..width {
            let cell = &buf[(x, y)];
            let s = cell.style();
            if Some(s) != last_style {
                let fg = s.fg.unwrap_or(ratatui::style::Color::Reset);
                line.push_str("\x1b[0m");
                match fg {
                    ratatui::style::Color::Reset => line.push_str("\x1b[39m"),
                    ratatui::style::Color::Rgb(r, g, b) => line.push_str(&format!("\x1b[38;2;{};{};{}m", r, g, b)),
                    ratatui::style::Color::Red | ratatui::style::Color::LightRed => line.push_str("\x1b[31m"),
                    ratatui::style::Color::Green | ratatui::style::Color::LightGreen => line.push_str("\x1b[32m"),
                    ratatui::style::Color::Yellow | ratatui::style::Color::LightYellow => line.push_str("\x1b[33m"),
                    ratatui::style::Color::Blue | ratatui::style::Color::LightBlue => line.push_str("\x1b[34m"),
                    ratatui::style::Color::Magenta | ratatui::style::Color::LightMagenta => line.push_str("\x1b[35m"),
                    ratatui::style::Color::Cyan | ratatui::style::Color::LightCyan => line.push_str("\x1b[36m"),
                    _ => line.push_str("\x1b[37m"),
                }
                // background
                if let Some(bg) = s.bg {
                    match bg {
                        ratatui::style::Color::Rgb(r, g, b) => line.push_str(&format!("\x1b[48;2;{};{};{}m", r, g, b)),
                        ratatui::style::Color::Yellow | ratatui::style::Color::LightYellow => line.push_str("\x1b[43m"),
                        ratatui::style::Color::Green | ratatui::style::Color::LightGreen => line.push_str("\x1b[42m"),
                        ratatui::style::Color::Red | ratatui::style::Color::LightRed => line.push_str("\x1b[41m"),
                        ratatui::style::Color::Blue | ratatui::style::Color::LightBlue => line.push_str("\x1b[44m"),
                        _ => line.push_str("\x1b[49m"),
                    };
                } else {
                    line.push_str("\x1b[49m");
                }
                last_style = Some(s);
            }
            line.push_str(cell.symbol());
        }
        line.push_str("\x1b[0m");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    print!("{}", out);
}
