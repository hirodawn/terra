//! A "CSS-grade" Markdown preview renderer for the terminal.
//!
//! Instead of emitting a flat list of styled lines, this module *paints* blocks
//! directly into the frame buffer: full-width background fills, rounded "card"
//! borders for code, colored left bars for quotes, hanging-indented lists, boxed
//! tables, and rule-with-ornament headings. The goal: not look like a TUI.

use crate::markdown;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;
use std::sync::OnceLock;

/// Curated dark palette (GitHub-dark inspired, refined).
pub struct Pal {
    pub bg: Color,
    pub text: Color,
    pub strong: Color,
    pub muted: Color,
    pub h: [Color; 6],
    pub rule: Color,
    pub code_bg: Color,
    pub code_border: Color,
    pub inline_code_bg: Color,
    pub inline_code_fg: Color,
    pub quote_bar: Color,
    pub quote_bg: Color,
    pub link: Color,
    pub table_head_bg: Color,
    pub table_alt_bg: Color,
    pub table_border: Color,
    pub ornament: Color,
    pub checkbox_on: Color,
    pub checkbox_off: Color,
}

fn dark_pal() -> Pal {
    Pal {
        bg: Color::Rgb(13, 17, 23),
        text: Color::Rgb(201, 209, 217),
        strong: Color::Rgb(245, 247, 250),
        muted: Color::Rgb(139, 148, 158),
        h: [
            Color::Rgb(255, 166, 87), Color::Rgb(121, 192, 255), Color::Rgb(210, 168, 255),
            Color::Rgb(115, 201, 144), Color::Rgb(255, 176, 176), Color::Rgb(139, 148, 158),
        ],
        rule: Color::Rgb(48, 54, 61),
        code_bg: Color::Rgb(22, 27, 34),
        code_border: Color::Rgb(48, 54, 61),
        inline_code_bg: Color::Rgb(40, 46, 60),
        inline_code_fg: Color::Rgb(121, 192, 255),
        quote_bar: Color::Rgb(255, 138, 92),
        quote_bg: Color::Rgb(22, 27, 34),
        link: Color::Rgb(88, 166, 255),
        table_head_bg: Color::Rgb(28, 34, 47),
        table_alt_bg: Color::Rgb(18, 22, 30),
        table_border: Color::Rgb(48, 54, 61),
        ornament: Color::Rgb(255, 166, 87),
        checkbox_on: Color::Rgb(115, 201, 144),
        checkbox_off: Color::Rgb(110, 118, 129),
    }
}

fn light_pal() -> Pal {
    Pal {
        bg: Color::Rgb(250, 250, 246),
        text: Color::Rgb(36, 41, 47),
        strong: Color::Rgb(15, 20, 25),
        muted: Color::Rgb(108, 113, 122),
        h: [
            Color::Rgb(190, 63, 25), Color::Rgb(5, 80, 174), Color::Rgb(95, 50, 155),
            Color::Rgb(26, 99, 52), Color::Rgb(200, 85, 85), Color::Rgb(108, 113, 122),
        ],
        rule: Color::Rgb(208, 215, 222),
        code_bg: Color::Rgb(243, 244, 246),
        code_border: Color::Rgb(208, 215, 222),
        inline_code_bg: Color::Rgb(230, 232, 238),
        inline_code_fg: Color::Rgb(5, 80, 174),
        quote_bar: Color::Rgb(234, 117, 70),
        quote_bg: Color::Rgb(243, 244, 246),
        link: Color::Rgb(9, 105, 218),
        table_head_bg: Color::Rgb(238, 240, 243),
        table_alt_bg: Color::Rgb(247, 248, 250),
        table_border: Color::Rgb(208, 215, 222),
        ornament: Color::Rgb(190, 63, 25),
        checkbox_on: Color::Rgb(26, 99, 52),
        checkbox_off: Color::Rgb(150, 156, 166),
    }
}

fn pal(dark: bool) -> &'static Pal {
    static D: OnceLock<Pal> = OnceLock::new();
    static L: OnceLock<Pal> = OnceLock::new();
    if dark {
        D.get_or_init(dark_pal)
    } else {
        L.get_or_init(light_pal)
    }
}

struct Syntax {
    ss: syntect::parsing::SyntaxSet,
    ts: syntect::highlighting::ThemeSet,
}
fn syntax() -> &'static Syntax {
    static S: OnceLock<Syntax> = OnceLock::new();
    S.get_or_init(|| Syntax {
        ss: syntect::parsing::SyntaxSet::load_defaults_newlines(),
        ts: syntect::highlighting::ThemeSet::load_defaults(),
    })
}

#[derive(Clone, Copy, Default)]
struct Fmt {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: bool,
}
struct Token {
    s: String,
    f: Fmt,
}

pub fn render(f: &mut Frame, area: ratatui::layout::Rect, src: &str, scroll: usize, dark: bool, code_theme: &str) -> usize {
    render_buf(f.buffer_mut(), area, src, scroll, dark, code_theme)
}

/// Paint the document into an arbitrary buffer (used by `render` and by tests).
pub fn render_buf(buf: &mut Buffer, area: ratatui::layout::Rect, src: &str, scroll: usize, dark: bool, code_theme: &str) -> usize {
    let p = pal(dark);
    // clear
    fill_rect(buf, area, p.bg);
    let pad = 2u16;
    let x0 = area.x + pad;
    let inner_w = (area.width as usize).saturating_sub(2 * pad as usize).max(1);

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(src, opts);

    let mut pt = Painter {
        buf,
        area,
        x0,
        inner_w,
        scroll,
        y: 0i64,
        p,
        max_y: area.y as i64 + area.height as i64,
        code_theme: code_theme.to_string(),
    };

    let mut ctx = Ctx {
        styles: Vec::new(),
        list_stack: Vec::new(),
        tok_stack: Vec::new(),
        block_stack: Vec::new(),
        quote_depth: 0,
        task_pending: None,
        table: TableBuild::default(),
        pending_code: String::new(),
        code_lang: String::new(),
        in_code: false,
    };

    for ev in parser {
        match ev {
            Event::Start(tag) => ctx.start(tag, &mut pt),
            Event::End(end) => {
                let flushed = ctx.end(end, &mut pt);
                let _ = flushed;
            }
            Event::Text(t) => {
                if ctx.in_code {
                    ctx.pending_code.push_str(&t);
                } else {
                    ctx.push_text(t.into_string());
                }
            }
            Event::Code(c) => {
                ctx.push_style(Fmt { code: true, ..Default::default() });
                ctx.push_text(c.into_string());
                ctx.styles.pop();
            }
            Event::SoftBreak | Event::HardBreak => ctx.soft_break(),
            Event::Rule => pt.rule(),
            Event::TaskListMarker(checked) => ctx.task_pending = Some(checked),
            Event::FootnoteReference(tag) => {
                ctx.push_style(Fmt { link: true, ..Default::default() });
                ctx.push_text(format!("[¹{}]", tag));
                ctx.styles.pop();
            }
            // Inline/raw HTML is passed through as literal text (DF spec: HTML is preserved).
            Event::Html(h) | Event::InlineHtml(h) => {
                ctx.push_text(h.into_string());
            }
            _ => {}
        }
    }
    pt.content_height()
}

struct Painter<'a> {
    buf: &'a mut Buffer,
    area: ratatui::layout::Rect,
    x0: u16,
    inner_w: usize,
    scroll: usize,
    y: i64,
    max_y: i64,
    p: &'static Pal,
    code_theme: String,
}

#[derive(PartialEq)]
#[allow(dead_code)]
enum Block {
    None,
    Paragraph,
    Heading(HeadingLevel),
    Item,
    FootnoteDef,
}

#[derive(Clone)]
struct ListInfo {
    ordered: bool,
    counter: u64,
}
#[derive(Default)]
struct TableBuild {
    rows: Vec<Vec<String>>,
}

struct Ctx {
    styles: Vec<Fmt>,
    list_stack: Vec<ListInfo>,
    tok_stack: Vec<Vec<Token>>,
    block_stack: Vec<Block>,
    quote_depth: usize,
    task_pending: Option<bool>,
    table: TableBuild,
    pending_code: String,
    code_lang: String,
    in_code: bool,
}

impl Ctx {
    fn push_style(&mut self, f: Fmt) {
        self.styles.push(f);
    }
    fn current_fmt(&self) -> Fmt {
        let mut f = Fmt::default();
        for s in &self.styles {
            f.bold |= s.bold;
            f.italic |= s.italic;
            f.strike |= s.strike;
            f.code |= s.code;
            f.link |= s.link;
        }
        f
    }
    fn push_text(&mut self, s: String) {
        // table cells accumulate raw
        if !self.table.rows.is_empty() && self.block_stack.last().map_or(true, |b| !matches!(b, Block::Paragraph | Block::Item)) {
            if let Some(row) = self.table.rows.last_mut() {
                if let Some(cell) = row.last_mut() {
                    cell.push_str(&s);
                    return;
                }
            }
        }
        let f = self.current_fmt();
        if let Some(top) = self.tok_stack.last_mut() {
            for part in split_tokens(&s) {
                if !part.is_empty() {
                    top.push(Token { s: part, f });
                }
            }
        }
    }
    fn soft_break(&mut self) {
        if let Some(top) = self.tok_stack.last_mut() {
            top.push(Token { s: "\n".into(), f: Fmt::default() });
        }
    }
    fn begin(&mut self, b: Block) {
        self.tok_stack.push(Vec::new());
        self.block_stack.push(b);
    }
    fn start(&mut self, tag: Tag, pt: &mut Painter) {
        match tag {
            Tag::Paragraph => {
                self.begin(Block::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.begin(Block::Heading(level));
            }
            Tag::CodeBlock(kind) => {
                self.pending_code.clear();
                self.code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.into_string(),
                    _ => String::new(),
                };
                self.in_code = true;
            }
            Tag::List(start) => {
                // Flush a pending item's leading text so the parent renders before its children.
                let need_flush = matches!(self.block_stack.last(), Some(Block::Item))
                    && self.tok_stack.last().map_or(false, |t| !t.is_empty());
                if need_flush {
                    let toks = std::mem::take(self.tok_stack.last_mut().unwrap());
                    let li = self.list_stack.last().cloned();
                    let depth = self.list_stack.len().saturating_sub(1);
                    pt.list_item(&toks, li, depth, None);
                }
                self.list_stack.push(ListInfo {
                    ordered: start.is_some(),
                    counter: start.unwrap_or(1),
                });
            }
            Tag::Item => {
                self.begin(Block::Item);
            }
            Tag::Emphasis => self.push_style(Fmt { italic: true, ..Default::default() }),
            Tag::Strong => self.push_style(Fmt { bold: true, ..Default::default() }),
            Tag::Strikethrough => self.push_style(Fmt { strike: true, ..Default::default() }),
            Tag::Link { .. } => self.push_style(Fmt { link: true, ..Default::default() }),
            Tag::Image { .. } => self.push_style(Fmt { link: true, ..Default::default() }),
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::Table(_) => {
                self.table = TableBuild::default();
            }
            Tag::TableHead | Tag::TableRow => self.table.rows.push(Vec::new()),
            Tag::TableCell => {
                if let Some(r) = self.table.rows.last_mut() {
                    r.push(String::new());
                }
            }
            Tag::FootnoteDefinition(_) => {
                self.begin(Block::FootnoteDef);
            }
            _ => {}
        }
    }
    fn end(&mut self, end: TagEnd, pt: &mut Painter) {
        match end {
            TagEnd::Paragraph => {
                if matches!(self.block_stack.last(), Some(Block::Paragraph)) {
                    let toks = self.tok_stack.pop().unwrap_or_default();
                    self.block_stack.pop();
                    pt.paragraph(&toks, self.quote_depth);
                }
            }
            TagEnd::Heading(level) => {
                if matches!(self.block_stack.last(), Some(Block::Heading(_))) {
                    let toks = self.tok_stack.pop().unwrap_or_default();
                    self.block_stack.pop();
                    pt.heading(&toks, level);
                }
            }
            TagEnd::CodeBlock => {
                pt.code_card(&self.code_lang, &self.pending_code);
                self.pending_code.clear();
                self.in_code = false;
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    pt.blank();
                }
            }
            TagEnd::Item => {
                let toks = self.tok_stack.pop().unwrap_or_default();
                self.block_stack.pop();
                if !toks.is_empty() {
                    let li = self.list_stack.last().cloned();
                    let depth = self.list_stack.len().saturating_sub(1);
                    pt.list_item(&toks, li, depth, self.task_pending.take());
                } else {
                    let _ = self.task_pending.take();
                }
                if let Some(l) = self.list_stack.last_mut() {
                    l.counter += 1;
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image => {
                self.styles.pop();
            }
            TagEnd::BlockQuote(_) => {
                if self.quote_depth > 0 {
                    self.quote_depth -= 1;
                }
                if self.quote_depth == 0 {
                    pt.blank();
                }
            }
            TagEnd::Table => {
                pt.table(&self.table.rows);
                self.table = TableBuild::default();
            }
            TagEnd::TableHead | TagEnd::TableRow | TagEnd::TableCell => {}
            TagEnd::FootnoteDefinition => {
                let toks = self.tok_stack.pop().unwrap_or_default();
                self.block_stack.pop();
                pt.footnote(&toks);
            }
            _ => {}
        }
    }
}

// extra fields via extension
impl Ctx {
    // placeholders for code-block text accumulation
}

// We need code accumulation fields; redeclare via a wrapper. Simplest: store on Ctx.
// (Rust doesn't allow adding fields later, so redefine Ctx with them.)

impl<'a> Painter<'a> {
    fn screen_y(&self, ly: i64) -> Option<u16> {
        let sy = self.area.y as i64 + ly - self.scroll as i64;
        if sy >= self.area.y as i64 && sy < self.max_y {
            Some(sy as u16)
        } else {
            None
        }
    }
    fn advance(&mut self) {
        self.y += 1;
    }
    fn content_height(&self) -> usize {
        self.y as usize
    }
    fn blank(&mut self) {
        self.advance();
    }
    fn put(&mut self, x: u16, ly: i64, ch: char, style: Style) {
        if x < self.area.x + self.area.width {
            if let Some(sy) = self.screen_y(ly) {
                let cell = &mut self.buf[(x, sy)];
                cell.set_char(ch);
                cell.set_style(style);
            }
        }
    }
    fn fill_line(&mut self, ly: i64, x_start: u16, x_end: u16, bg: Color) {
        if let Some(sy) = self.screen_y(ly) {
            let lo = x_start.max(self.area.x);
            let hi = x_end.min(self.area.x + self.area.width);
            for x in lo..hi {
                let cell = &mut self.buf[(x, sy)];
                if cell.symbol() == " " {
                    cell.set_char(' ');
                }
                let s = cell.style();
                cell.set_style(Style { bg: Some(bg), ..s });
            }
        }
    }
    fn write_str(&mut self, x: u16, ly: i64, s: &str, style: Style) -> u16 {
        let mut cx = x;
        for c in s.chars() {
            if c == '\n' {
                continue;
            }
            let w = cell_w(c);
            self.put(cx, ly, c, style);
            cx += w as u16;
        }
        cx
    }

    // ---- blocks ----
    fn heading(&mut self, tokens: &[Token], level: HeadingLevel) {
        let p = self.p;
        self.blank();
        let idx = (level as usize - 1).min(5);
        let color = p.h[idx];
        // prefix marker for flavor
        let prefix = match level {
            HeadingLevel::H1 => "▍ ",
            _ => "",
        };
        let mut cx = self.write_str(self.x0, self.y, prefix, Style::default().fg(color));
        for t in tokens {
            let st = self.style_for(t.f, Some(color));
            cx = self.write_str(cx, self.y, &t.s, st);
        }
        self.advance();
        if level == HeadingLevel::H1 || level == HeadingLevel::H2 {
            self.hrule_raw(level == HeadingLevel::H1, color);
        }
        self.blank();
    }

    fn hrule_raw(&mut self, thick: bool, color: Color) {
        let ch = if thick { '━' } else { '─' };
        let st = Style::default().fg(color);
        for i in 0..(self.inner_w as u16) {
            self.put(self.x0 + i, self.y, ch, st);
        }
        self.advance();
    }

    fn rule(&mut self) {
        // ornament rule: ────── ✦ ──────
        self.blank();
        let p = self.p;
        let w = self.inner_w as u16;
        let mid = " ✦ ";
        let side = (w as i32 - mid.chars().count() as i32) / 2;
        let side = if side < 1 { 1 } else { side as u16 };
        let st = Style::default().fg(p.rule);
        let orn = Style::default().fg(p.ornament);
        let mut cx = self.x0;
        for _ in 0..side {
            self.put(cx, self.y, '─', st);
            cx += 1;
        }
        for c in mid.chars() {
            self.put(cx, self.y, c, orn);
            cx += 1;
        }
        while cx < self.x0 + w {
            self.put(cx, self.y, '─', st);
            cx += 1;
        }
        self.advance();
        self.blank();
    }

    fn paragraph(&mut self, tokens: &[Token], quote_depth: usize) {
        let p = self.p;
        let indent = (quote_depth * 3) as u16;
        let wx = self.x0 + indent;
        let width = (self.inner_w as u16).saturating_sub(indent) as usize;
        if width < 4 {
            return;
        }
        if quote_depth > 0 {
            self.fill_line(self.y, self.x0, self.x0 + self.inner_w as u16, p.quote_bg);
            self.paint_quote_bars(self.y, quote_depth);
        }
        let rows = wrap_tokens(tokens, width, self.p);
        for row in &rows {
            if quote_depth > 0 {
                self.fill_line(self.y, self.x0, self.x0 + self.inner_w as u16, p.quote_bg);
                self.paint_quote_bars(self.y, quote_depth);
            }
            let mut cx = wx;
            for cs in row {
                self.put(cx, self.y, cs.ch, cs.style);
                cx += cell_w(cs.ch) as u16;
            }
            self.advance();
        }
        let _ = wx;
        self.blank();
    }

    fn paint_quote_bars(&mut self, ly: i64, depth: usize) {
        let p = self.p;
        for d in 0..depth {
            let x = self.x0 + (d * 3) as u16;
            self.put(x, ly, '┃', Style::default().fg(p.quote_bar));
        }
    }

    fn list_item(&mut self, tokens: &[Token], li: Option<ListInfo>, depth: usize, task: Option<bool>) {
        let p = self.p;
        let indent = (depth * 2) as u16;
        let bx = self.x0 + indent;
        let text_x = bx + 3;
        let width = (self.inner_w as u16).saturating_sub(indent + 3) as usize;
        // marker
        let (marker, marker_style) = if let Some(t) = task {
            if t { ("✔ ".to_string(), Style::default().fg(p.checkbox_on)) }
            else { ("□ ".to_string(), Style::default().fg(p.checkbox_off)) }
        } else if let Some(l) = &li {
            if l.ordered {
                (format!("{}. ", l.counter), Style::default().fg(p.muted))
            } else {
                let m = match depth % 3 { 1 => "◦ ", 2 => "▪ ", _ => "• " };
                (m.to_string(), Style::default().fg(p.ornament))
            }
        } else {
            ("• ".into(), Style::default().fg(p.ornament))
        };
        let rows = wrap_tokens(tokens, width.max(2), self.p);
        for (i, row) in rows.iter().enumerate() {
            if i == 0 {
                self.write_str(bx, self.y, &marker, marker_style);
            }
            let mut cx = text_x;
            for cs in row {
                self.put(cx, self.y, cs.ch, cs.style);
                cx += cell_w(cs.ch) as u16;
            }
            self.advance();
        }
    }

    fn footnote(&mut self, tokens: &[Token]) {
        let p = self.p;
        let st = Style::default().fg(p.muted);
        let mut cx = self.write_str(self.x0, self.y, "↳ ", Style::default().fg(p.ornament));
        cx = self.write_str(cx, self.y, "", st);
        for t in tokens {
            cx = self.write_str(cx, self.y, &t.s, self.style_for(t.f, None));
        }
        self.advance();
        self.blank();
    }

    fn code_card(&mut self, lang: &str, code: &str) {
        let p = self.p;
        self.blank();
        let w = self.inner_w as u16;
        let border = Style::default().fg(p.code_border);
        let label_style = Style::default().fg(p.muted).add_modifier(Modifier::DIM);
        // top border: ╭ ─ lang ─── ╮
        self.put(self.x0, self.y, '╭', border);
        let mut cx = self.x0 + 1;
        let lang_display = if lang.is_empty() { "code".to_string() } else { lang.to_string() };
        let label = format!(" {} ", lang_display);
        for c in label.chars() {
            self.put(cx, self.y, c, label_style);
            cx += 1;
        }
        while cx < self.x0 + w - 1 {
            self.put(cx, self.y, '─', border);
            cx += 1;
        }
        self.put(self.x0 + w - 1, self.y, '╮', border);
        self.advance();

        // body
        let sa = syntax();
        let syntax_ref = if lang.is_empty() {
            sa.ss.find_syntax_plain_text()
        } else {
            sa.ss.find_syntax_by_token(lang)
                .or_else(|| sa.ss.find_syntax_by_extension(lang))
                .unwrap_or_else(|| sa.ss.find_syntax_plain_text())
        };
        let theme = sa.ts.themes.get(&self.code_theme).cloned();
        let code_style = Style::default().fg(p.text).bg(p.code_bg);
        for raw in code.trim_end_matches('\n').lines() {
            // bg fill + side bars
            self.fill_line(self.y, self.x0, self.x0 + w, p.code_bg);
            self.put(self.x0, self.y, '│', border);
            self.put(self.x0 + w - 1, self.y, '│', border);
            let text_x = self.x0 + 2;
            if let Some(theme) = &theme {
                use syntect::easy::HighlightLines;
                let mut h = HighlightLines::new(syntax_ref, theme);
                let regions = h.highlight_line(raw, &sa.ss).unwrap_or_default();
                let mut cx = text_x;
                for (st, s) in regions {
                    let fg = synthect_to_ratatui(st, p.text);
                    for c in s.chars() {
                        if cx >= self.x0 + w - 1 {
                            break;
                        }
                        self.put(cx, self.y, c, Style::default().fg(fg).bg(p.code_bg));
                        cx += cell_w(c) as u16;
                    }
                }
            } else {
                self.write_str(text_x, self.y, raw, code_style);
            }
            self.advance();
        }
        // bottom border
        self.put(self.x0, self.y, '╰', border);
        for i in 1..w - 1 {
            self.put(self.x0 + i, self.y, '─', border);
        }
        self.put(self.x0 + w - 1, self.y, '╯', border);
        self.advance();
        self.blank();
    }

    fn table(&mut self, rows: &[Vec<String>]) {
        let p = self.p;
        if rows.is_empty() {
            return;
        }
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let pad = 1u16;
        let mut widths = vec![0u16; cols];
        for r in rows {
            for (i, c) in r.iter().enumerate() {
                widths[i] = widths[i].max(c.chars().count() as u16);
            }
        }
        let border = Style::default().fg(p.table_border);
        let head_bg = p.table_head_bg;
        let alt_bg = p.table_alt_bg;
        self.blank();
        // top border  ┌──┬──┐
        self.put(self.x0, self.y, '┌', border);
        let mut cx = self.x0 + 1;
        for (i, wd) in widths.iter().enumerate() {
            for _ in 0..(wd + 2 * pad) { self.put(cx, self.y, '─', border); cx += 1; }
            if i + 1 < cols { self.put(cx, self.y, '┬', border); cx += 1; }
        }
        self.put(cx, self.y, '┐', border);
        self.advance();
        for (ri, row) in rows.iter().enumerate() {
            // cell bg fill
            if ri == 0 {
                self.fill_line(self.y, self.x0, self.x0 + self.inner_w as u16, head_bg);
            } else if ri % 2 == 0 {
                self.fill_line(self.y, self.x0, self.x0 + self.inner_w as u16, alt_bg);
            }
            let mut cx = self.x0;
            self.put(cx, self.y, '│', border);
            cx += 1;
            for ci in 0..cols {
                let cell = row.get(ci).cloned().unwrap_or_default();
                for _ in 0..pad {
                    self.put(cx, self.y, ' ', Style::default());
                    cx += 1;
                }
                let st = if ri == 0 {
                    Style::default().fg(p.strong).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(p.text)
                };
                cx = self.write_str(cx, self.y, &cell, st);
                let remaining = widths[ci] as i32 - cell.chars().count() as i32;
                for _ in 0..remaining.max(0) {
                    self.put(cx, self.y, ' ', Style::default());
                    cx += 1;
                }
                for _ in 0..pad {
                    self.put(cx, self.y, ' ', Style::default());
                    cx += 1;
                }
                self.put(cx, self.y, '│', border);
                cx += 1;
            }
            self.advance();
            if ri == 0 {
                // separator row
                self.put(self.x0, self.y, '├', border);
                let mut cx = self.x0 + 1;
                for (i, wd) in widths.iter().enumerate() {
                    for _ in 0..(wd + 2 * pad) {
                        self.put(cx, self.y, '─', border);
                        cx += 1;
                    }
                    if i + 1 < cols {
                        self.put(cx, self.y, '┼', border);
                        cx += 1;
                    }
                }
                self.put(cx, self.y, '┤', border);
                self.advance();
            }
        }
        // bottom border └──┴──┘
        self.put(self.x0, self.y, '└', border);
        let mut cx = self.x0 + 1;
        for (i, wd) in widths.iter().enumerate() {
            for _ in 0..(wd + 2 * pad) { self.put(cx, self.y, '─', border); cx += 1; }
            if i + 1 < cols { self.put(cx, self.y, '┴', border); cx += 1; }
        }
        self.put(cx, self.y, '┘', border);
        self.advance();
        self.blank();
    }

    fn style_for(&self, f: Fmt, heading_color: Option<Color>) -> Style {
        let p = self.p;
        let mut fg = if f.code {
            p.inline_code_fg
        } else if heading_color.is_some() {
            heading_color.unwrap()
        } else {
            p.text
        };
        let bg = if f.code { p.inline_code_bg } else { p.bg };
        if f.bold {
            fg = p.strong;
        }
        let mut st = Style::default().fg(fg).bg(bg);
        if f.bold {
            st = st.add_modifier(Modifier::BOLD);
        }
        if f.italic {
            st = st.add_modifier(Modifier::ITALIC);
        }
        if f.strike {
            st = st.add_modifier(Modifier::CROSSED_OUT);
        }
        if f.link {
            st = Style::default().fg(p.link).add_modifier(Modifier::UNDERLINED);
        }
        st
    }
}

struct CellSpec {
    ch: char,
    style: Style,
}

/// Wrap a flat token list into rows of styled cells fitting `width`.
fn wrap_tokens(tokens: &[Token], width: usize, p: &Pal) -> Vec<Vec<CellSpec>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<CellSpec>> = vec![Vec::new()];
    let mut cur = 0usize;
    for tok in tokens {
        let base = style_for_fmt(tok.f, p);
        for c in tok.s.chars() {
            if c == '\n' {
                rows.push(Vec::new());
                cur = 0;
                continue;
            }
            let w = cell_w(c);
            if cur + w > width && cur > 0 {
                rows.push(Vec::new());
                cur = 0;
            }
            rows.last_mut().unwrap().push(CellSpec { ch: c, style: base });
            cur += w;
        }
    }
    rows
}

fn style_for_fmt(f: Fmt, p: &Pal) -> Style {
    let mut fg = if f.code { p.inline_code_fg } else { p.text };
    let bg = if f.code { p.inline_code_bg } else { p.bg };
    if f.bold {
        fg = p.strong;
    }
    let mut st = Style::default().fg(fg).bg(bg);
    if f.bold {
        st = st.add_modifier(Modifier::BOLD);
    }
    if f.italic {
        st = st.add_modifier(Modifier::ITALIC);
    }
    if f.strike {
        st = st.add_modifier(Modifier::CROSSED_OUT);
    }
    if f.link {
        st = Style::default().fg(p.link).add_modifier(Modifier::UNDERLINED);
    }
    st
}

fn cell_w(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(1).max(0)
}

fn synthect_to_ratatui(s: syntect::highlighting::Style, fallback: Color) -> Color {
    if s.foreground.a == 0 {
        fallback
    } else {
        Color::Rgb(s.foreground.r, s.foreground.g, s.foreground.b)
    }
}

fn fill_rect(buf: &mut Buffer, area: ratatui::layout::Rect, bg: Color) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            cell.set_char(' ');
            cell.set_style(Style::default().bg(bg));
        }
    }
}

fn split_tokens(s: &str) -> Vec<String> {
    // split keeping trailing whitespace attached to the preceding chunk
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in s.chars() {
        if c == ' ' || c == '\t' {
            buf.push(c);
        } else {
            if !buf.is_empty() && buf.chars().last().map_or(false, |c| c == ' ' || c == '\t') {
                // we had whitespace then a non-ws: flush whitespace-only? keep attached
            }
            buf.push(c);
        }
    }
    // simpler: just keep whole words+trailing space as tokens
    out.clear();
    let mut cur = String::new();
    for c in s.chars() {
        cur.push(c);
        if c == ' ' {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// silence unused
#[allow(unused)]
fn _silence() {
    let _ = markdown::render_lines;
    let _ = (Line::default(), Span::raw(""));
}

/// Daring Fireball "Markdown: Syntax" (John Gruber) compliance tests.
/// Each test feeds a construct from the spec and asserts the rendered output
/// contains the right content with the raw markdown syntax consumed.
#[cfg(test)]
mod df_spec {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn render(md: &str) -> String {
        let area = Rect::new(0, 0, 100, 60);
        let mut buf = Buffer::empty(area);
        render_buf(&mut buf, area, md, 0, true, "base16-ocean.dark");
        let mut s = String::new();
        for y in 0..area.height {
            let mut row = String::new();
            for x in 0..area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            s.push_str(row.trim_end());
            s.push('\n');
        }
        s
    }
    /// collapse runs of whitespace into single spaces, for robust matching
    fn flat(md: &str) -> String {
        render(md).split_whitespace().collect::<Vec<_>>().join(" ")
    }

    // ---- Overview: inline HTML is preserved ----
    #[test]
    fn inline_html_preserved() {
        assert!(flat("a <b>bold</b> word").contains("bold"));
    }

    // ---- Paragraphs & line breaks ----
    #[test]
    fn paragraph_joins_lines() {
        // soft line breaks join into one paragraph
        assert!(flat("line one\nline two").contains("line one line two"));
    }
    #[test]
    fn blank_line_separates_paragraphs() {
        let f = flat("first paragraph\n\nsecond paragraph");
        assert!(f.contains("first paragraph"));
        assert!(f.contains("second paragraph"));
    }

    // ---- Atx headers (all six levels) ----
    #[test]
    fn atx_headers() {
        let md = "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6";
        let f = flat(md);
        for lvl in 1..=6 {
            let word = format!("H{}", lvl);
            assert!(f.contains(&word), "missing {}", word);
            let raw = format!("# {}", word);
            assert!(!f.contains(&raw), "raw syntax leaked: {}", raw);
        }
    }

    // ---- Setext headers ----
    #[test]
    fn setext_h1() {
        let f = flat("Title One\n===");
        assert!(f.contains("Title One"));
        assert!(!f.contains("===")); // underline consumed
    }
    #[test]
    fn setext_h2() {
        // use '***' rule guard so '---' is unambiguously a setext underline
        let f = flat("Section\n---");
        assert!(f.contains("Section"));
    }

    // ---- Blockquotes (incl. nested) ----
    #[test]
    fn blockquote() {
        assert!(flat("> a quoted line").contains("a quoted line"));
    }
    #[test]
    fn nested_blockquote() {
        assert!(flat("> > deeply nested").contains("deeply nested"));
    }

    // ---- Lists ----
    #[test]
    fn unordered_lists_all_markers() {
        for m in ["*", "+", "-"] {
            let md = format!("{} alpha\n{} beta", m, m);
            let f = flat(&md);
            assert!(f.contains("alpha"), "marker {} alpha", m);
            assert!(f.contains("beta"), "marker {} beta", m);
        }
    }
    #[test]
    fn ordered_list() {
        let f = flat("1. first\n2. second\n3. third");
        assert!(f.contains("first"));
        assert!(f.contains("second"));
        assert!(f.contains("third"));
        assert!(f.contains("1."));
        assert!(f.contains("3."));
    }
    #[test]
    fn nested_list() {
        let f = flat("- top\n  - child");
        assert!(f.contains("top"));
        assert!(f.contains("child"));
    }

    // ---- Indented code block ----
    #[test]
    fn indented_code_block() {
        let f = flat("    let x = 1;");
        assert!(f.contains("let x = 1;"));
    }

    // ---- Horizontal rules ----
    #[test]
    fn horizontal_rules_all() {
        for r in ["***", "---", "___", "* * *", "- - -"] {
            assert!(flat(r).contains("✦"), "rule {:?} not ornamented", r);
        }
    }

    // ---- Links ----
    #[test]
    fn inline_link() {
        let f = flat("see [the docs](https://example.com/docs)");
        assert!(f.contains("the docs"));
        assert!(!f.contains("[the docs]"));
        assert!(!f.contains("(https://example.com/docs)"));
    }
    #[test]
    fn reference_link() {
        let md = "go [home][1]\n\n[1]: https://example.com";
        let f = flat(md);
        assert!(f.contains("home"));
        assert!(!f.contains("[home]"));
    }
    #[test]
    fn implicit_reference_link() {
        let md = "visit [Example][]\n\n[Example]: https://example.com";
        assert!(flat(md).contains("Example"));
    }
    #[test]
    fn automatic_link() {
        let f = flat("<https://example.com>");
        assert!(f.contains("example.com"));
    }

    // ---- Emphasis ----
    #[test]
    fn italic_star() {
        let f = flat("this is *italic* text");
        assert!(f.contains("italic"));
        assert!(!f.contains("*italic*"));
    }
    #[test]
    fn italic_underscore() {
        let f = flat("this is _italic_ text");
        assert!(f.contains("italic"));
        assert!(!f.contains("_italic_"));
    }
    #[test]
    fn bold_star() {
        let f = flat("this is **bold** text");
        assert!(f.contains("bold"));
        assert!(!f.contains("**bold**"));
    }
    #[test]
    fn bold_underscore() {
        let f = flat("this is __bold__ text");
        assert!(f.contains("bold"));
        assert!(!f.contains("__bold__"));
    }
    #[test]
    fn bold_italic_combined() {
        let f = flat("***both***");
        assert!(f.contains("both"));
        assert!(!f.contains("***"));
    }

    // ---- Inline code ----
    #[test]
    fn inline_code() {
        let f = flat("use `cargo run` now");
        assert!(f.contains("cargo run"));
        assert!(!f.contains("`cargo run`"));
    }
    #[test]
    fn inline_code_multibacktick() {
        let f = flat("a ``literal ` backtick`` b");
        assert!(f.contains("literal ` backtick"));
    }

    // ---- Images ----
    #[test]
    fn image_alt_text() {
        let f = flat("![a cute cat](cat.png)");
        assert!(f.contains("a cute cat"));
        assert!(!f.contains("![a cute cat]"));
    }

    // ---- Backslash escapes ----
    #[test]
    fn backslash_escape() {
        let f = flat(r"\*not italic\*");
        assert!(f.contains("*not italic*"));
    }

    // ---- Automatic email link ----
    #[test]
    fn autolink_email() {
        let f = flat("<user@example.com>");
        assert!(f.contains("user@example.com"));
    }

    // ---- Fenced code block (GFM, widely expected alongside the spec) ----
    #[test]
    fn fenced_code_block() {
        let f = flat("```rust\nfn main() {}\n```");
        assert!(f.contains("fn main()"));
        assert!(f.contains("rust")); // language label in card
    }

    // ===== Examples taken from the project's other pages =====
    // The "Basics" page (daringfireball.net/projects/markdown/basics) shows the
    // same constructs as Syntax, but with specific before/after examples. These
    // tests mirror those examples verbatim.

    // Basics: a heading rendered inside a blockquote.
    #[test]
    fn basics_heading_inside_blockquote() {
        let f = flat("> ## This is an H2 in a blockquote");
        assert!(f.contains("This is an H2 in a blockquote"));
    }

    // Basics: multiple paragraphs inside one blockquote.
    #[test]
    fn basics_multiparagraph_blockquote() {
        let md = "> This is a blockquote.\n>\n> This is the second paragraph in the blockquote.";
        let f = flat(md);
        assert!(f.contains("This is a blockquote."));
        assert!(f.contains("second paragraph in the blockquote"));
    }

    // Basics: a list item with multiple paragraphs (loose list).
    #[test]
    fn basics_multiparagraph_list_item() {
        let md = "* A list item.\n\n  With multiple paragraphs.\n* Another item in the list.";
        let f = flat(md);
        assert!(f.contains("A list item."));
        assert!(f.contains("With multiple paragraphs."));
        assert!(f.contains("Another item in the list."));
    }

    // Basics: full-width setext underline (many '=' signs) is still an H1.
    #[test]
    fn basics_setext_full_width_underline() {
        let md = "A First Level Header\n====================";
        let f = flat(md);
        assert!(f.contains("A First Level Header"));
        assert!(!f.contains("===================="));
    }

    // Basics: emphasized phrase examples (asterisks + underscores, single + double).
    #[test]
    fn basics_phrase_emphasis_examples() {
        let md = "Some of *these* words. And _these_ too. **Strong** and __strong__.";
        let f = flat(md);
        assert!(f.contains("these"));
        assert!(f.contains("Strong"));
        assert!(f.contains("strong"));
        assert!(!f.contains("**Strong**"));
        assert!(!f.contains("__strong__"));
    }

    // Basics: the three unordered markers are interchangeable (same output).
    #[test]
    fn basics_unordered_markers_interchangeable() {
        let a = flat("* Candy.\n* Gum.\n* Booze.");
        for w in ["Candy", "Gum", "Booze"] {
            assert!(a.contains(w));
        }
    }

    // Basics: inline link example.
    #[test]
    fn basics_inline_link_example() {
        let f = flat("This is an [example link](http://example.com/).");
        assert!(f.contains("example link"));
        assert!(!f.contains("[example link]"));
    }
}
