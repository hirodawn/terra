//! Application state and keyboard input handling for terra.

use crate::buffer::Buffer;
use ratatui::style::Color;

/// Which pane has keyboard focus.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Preview,
}

/// Built-in color themes for the editor chrome.
#[derive(Clone, Copy)]
pub enum Theme {
    Dark,
    Light,
    Ghost,
}

impl Theme {
    pub fn bg(self) -> Color {
        match self {
            Theme::Dark => Color::Reset,
            Theme::Light => Color::Rgb(245, 245, 240),
            Theme::Ghost => Color::Black,
        }
    }
    pub fn fg(self) -> Color {
        match self {
            Theme::Dark => Color::Gray,
            Theme::Light => Color::Rgb(40, 40, 48),
            Theme::Ghost => Color::DarkGray,
        }
    }
    pub fn accent(self) -> Color {
        match self {
            Theme::Dark => Color::LightBlue,
            Theme::Light => Color::Rgb(30, 100, 180),
            Theme::Ghost => Color::Cyan,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Ghost => "ghost",
        }
    }
}

pub struct App {
    pub buf: Buffer,
    pub title: String,
    pub focus: Focus,
    pub should_quit: bool,
    pub status: String,
    pub status_is_error: bool,
    pub theme: Theme,
    pub wrap: bool,
    pub show_help: bool,
    pub sync_preview: bool,
    pub outline_open: bool,
    pub outline_sel: usize,
    pub search_open: bool,
    pub search_query: String,
    pub last_query: String,
    pub command: String,
    pub in_command: bool,
    pub editor_scroll: usize,
    pub preview_scroll: usize,
    /// Editor inner rect, recorded each frame for mouse hit-testing.
    pub ei_x: u16,
    pub ei_y: u16,
    pub ei_w: u16,
    /// Preview pane rect for mouse hit-testing.
    pub pi_x: u16,
    pub pi_y: u16,
    pub pi_w: u16,
    /// Text selection state: start/end in absolute screen coords.
    pub sel_start: Option<(u16, u16)>,
    pub sel_end: Option<(u16, u16)>,
    /// Which pane the selection is in (0=editor, 1=preview).
    pub sel_pane: u8,
    /// Window sizes updated each frame by the UI.
    pub editor_height: usize,
    pub editor_width: usize,
    pub preview_height: usize,
    pub preview_width: usize,
    /// syntect assets
    pub syntax_set: syntect::parsing::SyntaxSet,
    pub theme_set: syntect::highlighting::ThemeSet,
    pub preview_theme: String,
    /// Preview cache: (revision, src_len) -> rendered lines.
    pub cached_rev: u64,
    pub cached_lines: Vec<ratatui::text::Line<'static>>,
    pub cache_width: usize,
    pub preview_content_height: usize,
    pub last_status_tick: std::time::Instant,
}

impl App {
    pub fn new(buf: Buffer, title: String) -> Self {
        let syntax_set = syntect::parsing::SyntaxSet::load_defaults_newlines();
        let theme_set = syntect::highlighting::ThemeSet::load_defaults();
        let preview_theme = "base16-ocean.dark".to_string();
        let title_for_status = title.clone();
        Self {
            buf,
            title,
            focus: Focus::Editor,
            should_quit: false,
            status: format!("{} — terra", title_for_status),
            status_is_error: false,
            theme: Theme::Dark,
            wrap: true,
            show_help: false,
            sync_preview: true,
            outline_open: false,
            outline_sel: 0,
            search_open: false,
            search_query: String::new(),
            last_query: String::new(),
            command: String::new(),
            in_command: false,
            editor_scroll: 0,
            preview_scroll: 0,
            ei_x: 0,
            ei_y: 0,
            ei_w: 0,
            pi_x: 0,
            pi_y: 0,
            pi_w: 0,
            sel_start: None,
            sel_end: None,
            sel_pane: 0,
            editor_height: 0,
            editor_width: 0,
            preview_height: 0,
            preview_width: 0,
            syntax_set,
            theme_set,
            preview_theme,
            cached_rev: u64::MAX,
            cached_lines: Vec::new(),
            cache_width: 0,
            preview_content_height: 0,
            last_status_tick: std::time::Instant::now(),
        }
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status = msg.to_string();
        self.status_is_error = false;
        self.last_status_tick = std::time::Instant::now();
    }

    pub fn set_error(&mut self, msg: &str) {
        self.status = msg.to_string();
        self.status_is_error = true;
        self.last_status_tick = std::time::Instant::now();
    }

    /// Ensure preview reflects buffer; recompute only when changed or width changed.
    pub fn ensure_preview(&mut self) {
        if self.cached_rev != self.buf.revision {
            self.cached_lines = crate::markdown::render_lines(
                &self.buf.text(),
                &self.syntax_set,
                &self.theme_set,
                &self.preview_theme,
            );
            self.cached_rev = self.buf.revision;
            self.cache_width = 0; // force rewrap
        }
    }

    pub fn switch_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Editor => Focus::Preview,
            Focus::Preview => Focus::Editor,
        };
        self.set_status(match self.focus {
            Focus::Editor => "edit",
            Focus::Preview => "preview (j/k to scroll)",
        });
    }

    pub fn cycle_theme(&mut self) {
        self.theme = match self.theme {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Ghost,
            Theme::Ghost => Theme::Dark,
        };
        self.preview_theme = match self.theme {
            Theme::Dark => "base16-ocean.dark".to_string(),
            Theme::Light => "InspiredGitHub".to_string(),
            Theme::Ghost => "base16-eighties.dark".to_string(),
        };
        self.cached_rev = u64::MAX; // force re-render
        self.set_status(&format!("theme: {}", self.theme.name()));
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.cache_width = 0;
        self.set_status(if self.wrap { "wrap: on" } else { "wrap: off" });
    }

    /// Scroll editor so cursor is visible.
    pub fn ensure_cursor_visible(&mut self) {
        let (row, _) = self.editor_cursor_display_row();
        if row < self.editor_scroll {
            self.editor_scroll = row;
        } else if row >= self.editor_scroll + self.editor_height {
            self.editor_scroll = row + 1 - self.editor_height;
        }
    }

    /// Returns (display_row, x_cell) of the cursor in the editor.
    pub fn editor_cursor_display_row(&self) -> (usize, usize) {
        let gutter = self.gutter_width();
        let avail = self.editor_width.saturating_sub(gutter + 1);
        let avail = avail.max(1);
        let mut row = 0usize;
        for i in 0..self.buf.cursor_line {
            let chars = self.buf.line_chars(i);
            row += if self.wrap {
                rows_for_line(&chars, avail)
            } else {
                1
            };
        }
        let chars = self.buf.line_chars(self.buf.cursor_line);
        if self.wrap {
            let (r, x) = locate_in_wrapped(&chars, self.buf.cursor_col, avail);
            (row + r, x)
        } else {
            let x = self.buf.cursor_col.min(self.editor_width);
            (row, x)
        }
    }

    pub fn gutter_width(&self) -> usize {
        let n = self.buf.line_count();
        let digits = n.to_string().len();
        digits.max(2) + 1
    }

    /// Move the editor cursor up by one *display* row (handles wrap).
    pub fn display_up(&mut self) {
        if !self.wrap {
            self.buf.up();
            return;
        }
        let (row, x) = self.editor_cursor_display_row();
        if row == 0 {
            self.buf.line_start();
            return;
        }
        // find target logical line & col
        let avail = self.editor_width.saturating_sub(self.gutter_width() + 1).max(1);
        let mut cur_row = 0usize;
        for i in 0..self.buf.line_count() {
            let chars = self.buf.line_chars(i);
            let nrows = rows_for_line(&chars, avail);
            if cur_row + nrows > row - 1 {
                // the row above is within this line if nrows>1 and cur_row<row
                if cur_row < row && nrows > 1 {
                    let within = row - 1 - cur_row;
                    let segs = segments(&chars, avail);
                    let seg = &segs[within.min(segs.len() - 1)];
                    self.buf.cursor_line = i;
                    self.buf.cursor_col = (seg.0 + x).min(seg.0 + seg.1);
                    return;
                }
                // otherwise it's the previous line
                self.buf.cursor_line = i.saturating_sub(1);
                let plen = self.buf.line_chars(self.buf.cursor_line).len();
                let segs = segments(&self.buf.line_chars(self.buf.cursor_line), avail);
                if let Some(last) = segs.last() {
                    self.buf.cursor_col = (last.0 + x).min(last.0 + last.1).min(plen);
                } else {
                    self.buf.cursor_col = plen;
                }
                return;
            }
            cur_row += nrows;
        }
    }

    pub fn display_down(&mut self) {
        if !self.wrap {
            self.buf.down();
            return;
        }
        let (row, x) = self.editor_cursor_display_row();
        let avail = self.editor_width.saturating_sub(self.gutter_width() + 1).max(1);
        let mut cur_row = 0usize;
        for i in 0..self.buf.line_count() {
            let chars = self.buf.line_chars(i);
            let nrows = rows_for_line(&chars, avail);
            if cur_row <= row && cur_row + nrows > row {
                let within = row - cur_row;
                let segs = segments(&chars, avail);
                if within + 1 < segs.len() {
                    let seg = &segs[within + 1];
                    self.buf.cursor_line = i;
                    self.buf.cursor_col = (seg.0 + x).min(seg.0 + seg.1);
                    return;
                }
                // go to next line
                if i + 1 < self.buf.line_count() {
                    self.buf.cursor_line = i + 1;
                    self.buf.cursor_col = x.min(self.buf.line_chars(self.buf.cursor_line).len());
                    return;
                }
            }
            cur_row += nrows;
        }
    }

    pub fn page_up(&mut self) {
        for _ in 0..self.editor_height.saturating_sub(1) {
            self.display_up();
        }
    }

    /// Position the cursor from a mouse click in the editor pane.
    pub fn click_editor(&mut self, column: u16, row: u16) {
        let gutter = self.gutter_width() as u16;
        let text_w = self.editor_width.saturating_sub(gutter as usize + 1).max(1);
        let display_row = (row as usize).saturating_sub(self.ei_y as usize) + self.editor_scroll;
        // walk lines accumulating display rows
        let mut cur = 0usize;
        for i in 0..self.buf.line_count() {
            let chars = self.buf.line_chars(i);
            let segs = segments(&chars, text_w);
            let nrows = segs.len().max(1);
            if cur + nrows > display_row {
                let within = display_row - cur;
                let seg = segs.get(within).copied().unwrap_or((0, 0));
                // cells from segment start to click
                let cells = (column as usize)
                    .saturating_sub(self.ei_x as usize)
                    .saturating_sub(gutter as usize);
                let mut col = seg.0;
                let mut used = 0usize;
                for &c in &chars[seg.0..(seg.0 + seg.1).min(chars.len())] {
                    if used + (char_cell(c) / 2).max(1) >= cells { break; }
                    used += char_cell(c);
                    col += 1;
                }
                self.buf.cursor_line = i;
                self.buf.cursor_col = col.min(self.line_len_public(i));
                self.ensure_cursor_visible();
                return;
            }
            cur += nrows;
        }
    }

    pub fn line_len_public(&self, i: usize) -> usize {
        self.buf.line_chars(i).len()
    }
    pub fn page_down(&mut self) {
        for _ in 0..self.editor_height.saturating_sub(1) {
            self.display_down();
        }
    }

    /// Move cursor to a specific line (1-indexed).
    pub fn goto_line(&mut self, n: usize) {
        if self.buf.line_count() == 0 {
            return;
        }
        self.buf.cursor_line = n.saturating_sub(1).min(self.buf.line_count() - 1);
        self.buf.cursor_col = 0;
        self.ensure_cursor_visible();
        self.set_status(&format!("→ line {}", self.buf.cursor_line + 1));
    }

    /// Proportionally scroll the preview to follow the editor cursor.
    pub fn sync_preview_to_cursor(&mut self) {
        if !self.sync_preview {
            return;
        }
        let ratio = (self.buf.cursor_line as f64) / (self.buf.line_count().max(1) as f64);
        let max = self.preview_content_height.saturating_sub(self.preview_height);
        self.preview_scroll = ((ratio * max as f64) as usize).min(max);
    }

    /// Return (line_index, level, title) for every heading in the buffer.
    pub fn headings(&self) -> Vec<(usize, usize, String)> {
        let mut out = Vec::new();
        for (i, line) in self.buf.lines.iter().enumerate() {
            let t = line.trim_start();
            if t.starts_with('#') {
                let level = t.chars().take_while(|c| *c == '#').count().min(6);
                if level > 0 {
                    let rest = t[level..].trim_start();
                    // skip a heading that's actually inside a code fence (cheap heuristic)
                    out.push((i, level, rest.to_string()));
                }
            }
        }
        out
    }

    pub fn outline_down(&mut self) {
        let h = self.headings();
        if !h.is_empty() {
            self.outline_sel = (self.outline_sel + 1).min(h.len() - 1);
        }
    }
    pub fn outline_up(&mut self) {
        self.outline_sel = self.outline_sel.saturating_sub(1);
    }
    pub fn outline_jump(&mut self) {
        let h = self.headings();
        if let Some((line, _, _)) = h.get(self.outline_sel) {
            let l = *line;
            self.goto_line(l + 1);
            self.outline_open = false;
        }
    }

    /// Find next line containing `query` (case-insensitive) starting after the cursor.
    pub fn find_next(&mut self, query: &str, forward: bool) {
        if query.is_empty() {
            return;
        }
        let q = query.to_lowercase();
        let n = self.buf.line_count();
        if n == 0 {
            return;
        }
        let start = self.buf.cursor_line;
        let idx = start;
        for step in 0..n {
            let i = if forward {
                (start + step + 1) % n
            } else {
                (start + n - step - 1) % n
            };
            if self.buf.lines[i].to_lowercase().contains(&q) {
                let col = self.buf.lines[i].to_lowercase().find(&q).unwrap_or(0);
                self.buf.cursor_line = i;
                self.buf.cursor_col = col;
                self.ensure_cursor_visible();
                self.sync_preview_to_cursor();
                self.set_status(&format!("match at line {}", i + 1));
                return;
            }
            let _ = idx;
        }
        self.set_status("no match");
    }
}

/// Number of display rows a line of chars will occupy at `width` cells.
pub fn rows_for_line(chars: &[char], width: usize) -> usize {
    let segs = segments(chars, width);
    segs.len().max(1)
}

fn char_cell(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(0)
}

/// Break a line into (start_col, length_in_cells) segments that fit `width`.
pub fn segments(chars: &[char], width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let mut out = Vec::new();
    if chars.is_empty() {
        return vec![(0, 0)];
    }
    let mut start = 0usize;
    let mut cells = 0usize;
    let mut last_word_start = None;
    let mut last_word_cells = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        let w = char_width(c);
        if cells + w > width {
            // need to break
            if start < i {
                out.push((start, cells));
            }
            // word-wrap: if we were mid-word, the break already happened
            start = i;
            cells = w;
            last_word_start = None;
            last_word_cells = w;
            continue;
        }
        if c == ' ' {
            last_word_start = None;
        } else if last_word_start.is_none() {
            last_word_start = Some(i);
            last_word_cells = w;
        } else {
            last_word_cells += w;
        }
        cells += w;
    }
    if start < chars.len() || cells > 0 {
        out.push((start, cells));
    }
    let _ = last_word_start;
    let _ = last_word_cells;
    if out.is_empty() {
        vec![(0, 0)]
    } else {
        out
    }
}

fn char_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(0)
}

/// Given a cursor column, find (row_index_in_line, x_offset_in_cells).
fn locate_in_wrapped(chars: &[char], col: usize, width: usize) -> (usize, usize) {
    let segs = segments(chars, width);
    let acc = 0usize;
    for (ri, (s, _len)) in segs.iter().enumerate() {
        let next_start = if ri + 1 < segs.len() {
            segs[ri + 1].0
        } else {
            chars.len() + 1
        };
        if col < next_start {
            // within this segment
            let mut x = 0;
            for &c in &chars[*s..col.min(chars.len())] {
                x += char_width(c);
            }
            return (ri, x);
        }
        let _ = acc;
    }
    (segs.len() - 1, 0)
}
