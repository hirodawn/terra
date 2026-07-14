//! Convert Markdown into styled ratatui lines for the preview pane.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use syntect::highlighting::{Style as SyStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Build the line model from the source markdown text. Each item is a
/// fully-styled logical line. The UI layer is responsible for wrapping.
pub fn render_lines(src: &str, ss: &SyntaxSet, ts: &ThemeSet, theme: &str) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(src, opts);

    let mut ctx = Ctx {
        out: Vec::new(),
        styles: Vec::new(),
        list_stack: Vec::new(),
        item_pending: false,
        in_code_block: false,
        code_lang: String::new(),
        code_buf: String::new(),
        table_rows: Vec::new(),
        in_table: false,
        quote_depth: 0,
        task_marker: None,
        footnote_tag: None,
        ss,
        ts,
        theme,
    };

    for event in parser {
        ctx.handle(event);
    }

    if ctx.out.is_empty() {
        ctx.out.push(Line::from(Span::raw(" ")));
    }
    ctx.out
}

#[derive(Clone, Copy, PartialEq)]
enum In {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Image,
}

struct Ctx<'a> {
    out: Vec<Line<'static>>,
    styles: Vec<In>,
    list_stack: Vec<(bool, u64)>, // (ordered, next_number) per nesting level
    item_pending: bool,
    in_code_block: bool,
    code_lang: String,
    code_buf: String,
    table_rows: Vec<Vec<String>>,
    in_table: bool,
    quote_depth: usize,
    ss: &'a SyntaxSet,
    ts: &'a ThemeSet,
    theme: &'a str,
    task_marker: Option<bool>,
    footnote_tag: Option<String>,
}

impl<'a> Ctx<'a> {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(end) => self.end(end),
            Event::Text(t) => {
                if self.in_code_block {
                    self.code_buf.push_str(&t);
                } else if self.in_table {
                    if let Some(row) = self.table_rows.last_mut() {
                        if let Some(cell) = row.last_mut() {
                            cell.push_str(t.as_ref());
                        } else {
                            row.push(t.into_string());
                        }
                    }
                } else {
                    self.emit_text(&t);
                }
            }
            Event::Code(c) => {
                self.styles.push(In::Code);
                self.emit_text(&c);
                self.styles.pop();
            }
            Event::SoftBreak | Event::HardBreak => {
                if !self.in_code_block {
                    self.new_line_with_prefix();
                }
            }
            Event::Rule => {
                self.out.push(Line::styled(
                    "────────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                ));
                self.blank();
            }
            Event::TaskListMarker(checked) => {
                self.task_marker = Some(checked);
            }
            Event::FootnoteReference(tag) => {
                if let Some(last) = self.out.last_mut() {
                    last.spans.push(Span::styled(
                        format!("[¹{}]", tag.as_ref()),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ));
                }
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.new_logical_line();
            }
            Tag::Heading { level, .. } => {
                self.new_logical_line();
                let s = match level {
                    pulldown_cmark::HeadingLevel::H1 => Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    pulldown_cmark::HeadingLevel::H2 => {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    }
                    pulldown_cmark::HeadingLevel::H3 => {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    }
                    pulldown_cmark::HeadingLevel::H4 => {
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                    }
                    pulldown_cmark::HeadingLevel::H5 => {
                        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
                    }
                    pulldown_cmark::HeadingLevel::H6 => Style::default().add_modifier(Modifier::BOLD),
                };
                let prefix = "#".repeat(level as usize);
                self.out.push(Line::from(Span::styled(format!("{} ", prefix), s)));
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                self.code_buf.clear();
                self.code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.into_string(),
                    _ => String::new(),
                };
            }
            Tag::List(start) => {
                let ordered = start.is_some();
                let next = start.unwrap_or(1);
                self.list_stack.push((ordered, next));
            }
            Tag::Item => {
                self.item_pending = true;
                self.new_logical_line();
            }
            Tag::Emphasis => self.styles.push(In::Italic),
            Tag::Strong => self.styles.push(In::Bold),
            Tag::Strikethrough => self.styles.push(In::Strike),
            Tag::Link { .. } => self.styles.push(In::Link),
            Tag::Image { .. } => {
                self.styles.push(In::Image);
                if let Some(last) = self.out.last_mut() {
                    last.spans.push(Span::styled("🖼 ".to_string(), Style::default().fg(Color::Magenta)));
                }
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
            }
            Tag::FootnoteDefinition(tag) => {
                self.new_logical_line();
                if let Some(last) = self.out.last_mut() {
                    last.spans.push(Span::styled(
                        format!("[^{}]: ", tag.as_ref()),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ));
                }
            }
            Tag::Table(_) => {
                self.in_table = true;
                self.table_rows.clear();
            }
            Tag::TableHead | Tag::TableRow => {
                self.table_rows.push(Vec::new());
            }
            Tag::TableCell => {
                if let Some(row) = self.table_rows.last_mut() {
                    row.push(String::new());
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph | TagEnd::Heading(_) => {
                self.blank();
            }
            TagEnd::CodeBlock => {
                self.flush_code_block();
                self.in_code_block = false;
                self.blank();
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => {
                self.item_pending = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                self.styles.pop();
            }
            TagEnd::BlockQuote(_) => {
                if self.quote_depth > 0 {
                    self.quote_depth -= 1;
                }
                if self.quote_depth == 0 {
                    self.blank();
                }
            }
            TagEnd::FootnoteDefinition => {
                self.footnote_tag = None;
                self.blank();
            }
            TagEnd::Table | TagEnd::TableHead | TagEnd::TableRow | TagEnd::TableCell => {
                if matches!(end, TagEnd::Table) {
                    self.flush_table();
                    self.in_table = false;
                    self.blank();
                }
            }
            _ => {}
        }
    }

    fn current_style(&self) -> Style {
        let mut s = Style::default();
        let mut is_code = false;
        for m in &self.styles {
            match m {
                In::Bold => s = s.add_modifier(Modifier::BOLD),
                In::Italic => s = s.add_modifier(Modifier::ITALIC),
                In::Strike => s = s.add_modifier(Modifier::CROSSED_OUT),
                In::Code => {
                    is_code = true;
                }
                In::Link => s = s.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED),
                In::Image => s = s.fg(Color::Magenta),
            }
        }
        if is_code {
            s = s
                .fg(Color::LightGreen)
                .bg(Color::Black);
        }
        if self.quote_depth > 0 {
            s = s.fg(Color::DarkGray);
        }
        s
    }

    fn emit_text(&mut self, t: &str) {
        let style = self.current_style();
        // write prefix (list bullet / quote) on first text of an item
        if self.item_pending {
            let prefix = self.make_item_prefix();
            if let Some(last) = self.out.last_mut() {
                if let Some(checked) = self.task_marker.take() {
                    let mark = if checked { "☑ " } else { "☐ " };
                    last.spans.push(Span::styled(mark.to_string(), Style::default().fg(if checked { Color::Green } else { Color::Gray })));
                }
                last.spans.push(Span::styled(prefix, Style::default().fg(Color::Gray)));
            }
            self.item_pending = false;
        }
        for (i, part) in t.split('\n').enumerate() {
            if i > 0 {
                self.new_line_with_prefix();
            }
            if !part.is_empty() {
                if let Some(last) = self.out.last_mut() {
                    last.spans.push(Span::styled(part.to_string(), style));
                }
            }
        }
    }

    fn make_item_prefix(&mut self) -> String {
        let depth = self.list_stack.len();
        let indent: String = "  ".repeat(depth.saturating_sub(1));
        if let Some((ordered, next)) = self.list_stack.last_mut() {
            if *ordered {
                let n = *next;
                *next += 1;
                format!("{}{}. ", indent, n)
            } else {
                format!("{}• ", indent)
            }
        } else {
            String::new()
        }
    }

    fn new_line_with_prefix(&mut self) {
        let mut line = Line::default();
        if self.quote_depth > 0 {
            let q = "│ ".repeat(self.quote_depth);
            line.spans.push(Span::styled(q, Style::default().fg(Color::Blue)));
        }
        self.out.push(line);
    }

    fn new_logical_line(&mut self) {
        // start a fresh line, possibly with quote prefix
        let mut line = Line::default();
        if self.quote_depth > 0 {
            let q = "│ ".repeat(self.quote_depth);
            line.spans.push(Span::styled(q, Style::default().fg(Color::Blue)));
        }
        // if this is the very first line, push; else ensure previous line exists
        if self.out.is_empty() {
            self.out.push(line);
        } else {
            // continue on the last pushed line if it's empty (no spans beyond prefix)
            self.out.push(line);
        }
    }

    fn blank(&mut self) {
        self.out.push(Line::raw(""));
    }

    fn flush_code_block(&mut self) {
        let code = std::mem::take(&mut self.code_buf);
        let syntax = if self.code_lang.is_empty() {
            self.ss.find_syntax_plain_text()
        } else {
            self.ss
                .find_syntax_by_token(&self.code_lang)
                .or_else(|| self.ss.find_syntax_by_extension(&self.code_lang))
                .unwrap_or_else(|| self.ss.find_syntax_plain_text())
        };
        let theme = self.ts.themes.get(self.theme).cloned();
        let mut line_nums = false;
        let _ = &mut line_nums;

        if let Some(theme) = theme {
            use syntect::easy::HighlightLines;
            use syntect::highlighting as syn;
            let mut h = HighlightLines::new(syntax, &theme);
            for raw in code.lines() {
                let regions: Vec<(SyStyle, &str)> =
                    syntect::easy::HighlightLines::highlight_line(&mut h, raw, self.ss)
                        .unwrap_or_default();
                let spans: Vec<Span> = regions
                    .into_iter()
                    .map(|(st, s)| Span::styled(s.to_string(), synthect_style_to_ratatui(st)))
                    .collect();
                let mut full: Vec<Span> = vec![Span::styled(" ", Style::default().bg(Color::Black))];
                full.extend(spans);
                self.out.push(Line::from(full));
            }
            let _ = syn::FontStyle::BOLD; // touch import
        } else {
            for raw in code.lines() {
                self.out.push(Line::styled(
                    format!(" {}", raw),
                    Style::default().fg(Color::Gray).bg(Color::Black),
                ));
            }
        }
    }

    fn flush_table(&mut self) {
        // Very simple ASCII table rendering.
        let rows = std::mem::take(&mut self.table_rows);
        if rows.is_empty() {
            return;
        }
        // column widths
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let mut widths = vec![0usize; cols];
        for r in &rows {
            for (i, c) in r.iter().enumerate() {
                widths[i] = widths[i].max(c.chars().count());
            }
        }
        let border_top: String = {
            let mut s = String::from("┌");
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                if i + 1 < widths.len() {
                    s.push('┬');
                }
            }
            s.push('┐');
            s
        };
        self.out.push(Line::styled(border_top, Style::default().fg(Color::DarkGray)));
        for (ri, r) in rows.iter().enumerate() {
            let mut line_spans: Vec<Span> = vec![Span::styled("│", Style::default().fg(Color::DarkGray))];
            for (i, w) in widths.iter().enumerate() {
                let cell = r.get(i).cloned().unwrap_or_default();
                let pad = w.saturating_sub(cell.chars().count());
                let style = if ri == 0 {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                line_spans.push(Span::raw(" "));
                line_spans.push(Span::styled(cell, style));
                line_spans.push(Span::raw(" ".repeat(pad)));
                line_spans.push(Span::raw(" "));
                line_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
                let _ = i;
            }
            self.out.push(Line::from(line_spans));
            if ri == 0 {
                let mid: String = {
                    let mut s = String::from("├");
                    for (i, w) in widths.iter().enumerate() {
                        s.push_str(&"─".repeat(w + 2));
                        if i + 1 < widths.len() {
                            s.push('┼');
                        }
                    }
                    s.push('┤');
                    s
                };
                self.out.push(Line::styled(mid, Style::default().fg(Color::DarkGray)));
            }
        }
        let border_bot: String = {
            let mut s = String::from("└");
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                if i + 1 < widths.len() {
                    s.push('┴');
                }
            }
            s.push('┘');
            s
        };
        self.out.push(Line::styled(border_bot, Style::default().fg(Color::DarkGray)));
    }
}

fn synthect_style_to_ratatui(s: SyStyle) -> Style {
    let mut out = Style::default();
    if s.foreground.a != 0 {
        out = out.fg(Color::Rgb(
            s.foreground.r,
            s.foreground.g,
            s.foreground.b,
        ));
    }
    if s.background.a != 0 {
        out = out.bg(Color::Rgb(s.background.r, s.background.g, s.background.b));
    }
    if s.font_style.contains(syntect::highlighting::FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(syntect::highlighting::FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if s.font_style.contains(syntect::highlighting::FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}
