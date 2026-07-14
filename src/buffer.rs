//! Line-based text buffer with cursor and editing operations.

use std::cmp::min;

/// A piece of text represented as a vector of lines. Each `String` holds the
/// contents of a single line *without* a trailing newline.
#[derive(Clone)]
pub struct Buffer {
    pub lines: Vec<String>,
    /// Logical cursor: (line, char-column).
    pub cursor_line: usize,
    pub cursor_col: usize,
    /// Desired visual column when moving vertically across short lines.
    desired_col: Option<usize>,
    pub dirty: bool,
    pub path: Option<std::path::PathBuf>,
    /// Modification generation, bumped on every edit (used for preview cache).
    pub revision: u64,
    history: Vec<Snap>,
    future: Vec<Snap>,
}

#[derive(Clone)]
struct Snap {
    text: String,
    line: usize,
    col: usize,
    group: Option<&'static str>,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(String::from).collect()
        };
        Self {
            lines,
            cursor_line: 0,
            cursor_col: 0,
            desired_col: None,
            dirty: false,
            path: None,
            revision: 0,
            history: Vec::new(),
            future: Vec::new(),
        }
    }

    pub fn from_path(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path)?;
        let mut buf = Buffer::new(&text);
        buf.path = Some(path);
        buf.dirty = false;
        Ok(buf)
    }

    pub fn text(&self) -> String {
        let mut s = String::with_capacity(self.lines.iter().map(|l| l.len() + 1).sum());
        for (i, l) in self.lines.iter().enumerate() {
            s.push_str(l);
            if i + 1 < self.lines.len() {
                s.push('\n');
            }
        }
        s
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(p) = &self.path {
            std::fs::write(p, self.text())?;
            self.dirty = false;
        }
        Ok(())
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Record the current state for undo. `group` lets consecutive inserts coalesce
    /// into a single undo step.
    fn checkpoint(&mut self, group: Option<&'static str>) {
        if group == Some("insert") {
            if let Some(last) = self.history.last() {
                if last.group == Some("insert")
                    && last.line == self.cursor_line
                    && last.col + 1 == self.cursor_col
                {
                    return; // coalesce
                }
            }
        }
        self.future.clear();
        self.history.push(Snap {
            text: self.text(),
            line: self.cursor_line,
            col: self.cursor_col,
            group,
        });
        if self.history.len() > 256 {
            self.history.remove(0);
        }
    }

    /// Undo the last change group.
    pub fn undo(&mut self) {
        if self.history.is_empty() {
            return;
        }
        self.future.push(Snap {
            text: self.text(),
            line: self.cursor_line,
            col: self.cursor_col,
            group: None,
        });
        let snap = self.history.pop().unwrap();
        self.restore(snap);
    }

    /// Redo a previously undone change.
    pub fn redo(&mut self) {
        if self.future.is_empty() {
            return;
        }
        self.history.push(Snap {
            text: self.text(),
            line: self.cursor_line,
            col: self.cursor_col,
            group: None,
        });
        let snap = self.future.pop().unwrap();
        self.restore(snap);
    }

    fn restore(&mut self, snap: Snap) {
        self.lines = if snap.text.is_empty() {
            vec![String::new()]
        } else {
            snap.text.split('\n').map(String::from).collect()
        };
        self.cursor_line = snap.line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = snap.col.min(self.line_len(self.cursor_line));
        self.desired_col = None;
        self.touch();
    }

    fn line_len(&self, idx: usize) -> usize {
        self.lines.get(idx).map(|l| l.chars().count()).unwrap_or(0)
    }

    #[allow(dead_code)]
    fn clamp_cursor(&mut self) {
        if self.cursor_line >= self.lines.len() {
            self.cursor_line = self.lines.len() - 1;
        }
        let max_col = self.line_len(self.cursor_line);
        if self.cursor_col > max_col {
            self.cursor_col = max_col;
        }
    }
    /// Move cursor left one character.
    pub fn left(&mut self) {
        self.desired_col = None;
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.line_len(self.cursor_line);
        }
    }

    pub fn right(&mut self) {
        self.desired_col = None;
        let max_col = self.line_len(self.cursor_line);
        if self.cursor_col < max_col {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    pub fn up(&mut self) {
        if self.cursor_line > 0 {
            if self.desired_col.is_none() {
                self.desired_col = Some(self.cursor_col);
            }
            self.cursor_line -= 1;
            let max_col = self.line_len(self.cursor_line);
            self.cursor_col = min(self.desired_col.unwrap(), max_col);
        }
    }

    pub fn down(&mut self) {
        if self.cursor_line + 1 < self.lines.len() {
            if self.desired_col.is_none() {
                self.desired_col = Some(self.cursor_col);
            }
            self.cursor_line += 1;
            let max_col = self.line_len(self.cursor_line);
            self.cursor_col = min(self.desired_col.unwrap(), max_col);
        }
    }

    pub fn line_start(&mut self) {
        self.desired_col = None;
        self.cursor_col = 0;
    }

    pub fn line_end(&mut self) {
        self.desired_col = None;
        self.cursor_col = self.line_len(self.cursor_line);
    }

    /// Move cursor by one word to the right. Returns nothing.
    pub fn word_forward(&mut self) {
        self.desired_col = None;
        let line = &self.lines[self.cursor_line];
        let chars: Vec<char> = line.chars().collect();
        let mut i = self.cursor_col;
        // skip non-word then word
        while i < chars.len() && !chars[i].is_alphanumeric() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_alphanumeric() {
            i += 1;
        }
        self.cursor_col = i;
    }

    pub fn word_back(&mut self) {
        self.desired_col = None;
        let line = &self.lines[self.cursor_line];
        let chars: Vec<char> = line.chars().collect();
        if self.cursor_col == 0 {
            return;
        }
        let mut i = self.cursor_col;
        while i > 0 && (i - 1 < chars.len()) && !chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && (i - 1 < chars.len()) && chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        self.cursor_col = i;
    }

    /// Insert a character at the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.checkpoint(Some("insert"));
        self.desired_col = None;
        let line = &mut self.lines[self.cursor_line];
        let mut new_line = String::with_capacity(line.len() + 4);
        let mut col = 0;
        for ch in line.chars() {
            if col == self.cursor_col {
                new_line.push(c);
            }
            new_line.push(ch);
            col += 1;
        }
        if col == self.cursor_col {
            new_line.push(c);
        }
        self.cursor_col += 1;
        self.lines[self.cursor_line] = new_line;
        self.touch();
    }

    /// Insert raw text which may contain newlines.
    pub fn insert_text(&mut self, text: &str) {
        self.checkpoint(Some("insert"));
        if text.is_empty() {
            return;
        }
        let parts: Vec<&str> = text.split('\n').collect();
        if parts.len() == 1 {
            for c in text.chars() {
                self.insert_char(c);
            }
            return;
        }
        // split current line at cursor
        let cur = self.lines[self.cursor_line].clone();
        let chars: Vec<char> = cur.chars().collect();
        let before: String = chars[..self.cursor_col].iter().collect();
        let after: String = chars[self.cursor_col..].iter().collect();
        // first part appended to before
        let mut first = before;
        first.push_str(parts[0]);
        self.lines[self.cursor_line] = first;
        // middle lines
        let mut line_idx = self.cursor_line;
        for (k, p) in parts[1..].iter().enumerate() {
            if k + 1 == parts.len() - 1 {
                // last: combine with `after`
                let mut last = String::new();
                last.push_str(p);
                last.push_str(&after);
                self.lines.insert(line_idx + 1, last);
                self.cursor_line = line_idx + 1;
                self.cursor_col = p.chars().count();
            } else {
                self.lines.insert(line_idx + 1, p.to_string());
                line_idx += 1;
            }
        }
        self.desired_col = None;
        self.touch();
    }

    pub fn insert_newline(&mut self) {
        self.insert_text("\n");
    }

    /// Duplicate the current line, placing the copy below. Cursor moves to the copy.
    pub fn duplicate_line(&mut self) {
        self.checkpoint(None);
        let cur = self.lines[self.cursor_line].clone();
        self.lines.insert(self.cursor_line + 1, cur);
        self.cursor_line += 1;
        self.desired_col = None;
        self.touch();
    }

    /// Delete the current line. The buffer always keeps at least one (empty) line.
    pub fn delete_line(&mut self) {
        self.checkpoint(None);
        if self.lines.len() <= 1 {
            self.lines[0].clear();
            self.cursor_col = 0;
            self.desired_col = None;
            self.touch();
            return;
        }
        self.lines.remove(self.cursor_line);
        if self.cursor_line >= self.lines.len() {
            self.cursor_line = self.lines.len() - 1;
        }
        self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
        self.desired_col = None;
        self.touch();
    }

    /// Smart newline: if the current line is a list item, continue the list on
    /// the new line with the right prefix (and an incremented number for ordered
    /// lists). If the item is empty, end the list instead (outdent).
    pub fn insert_newline_smart(&mut self) {
        let line = self.lines[self.cursor_line].clone();
        if let Some(prefix) = list_prefix(&line) {
            let content = &line[prefix.orig_len..];
            if content.trim().is_empty() {
                // empty item: clear the prefix and just break the line
                self.lines[self.cursor_line] = String::new();
                self.cursor_col = 0;
                self.insert_newline();
                return;
            }
            self.insert_newline();
            self.insert_text(&prefix.next);
        } else {
            // auto-indent: copy leading whitespace from the current line
            let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            self.insert_newline();
            if !indent.is_empty() {
                self.insert_text(&indent);
            }
        }
    }

    /// Delete char before cursor (backspace).
    pub fn backspace(&mut self) {
        self.checkpoint(None);
        self.desired_col = None;
        if self.cursor_col == 0 {
            if self.cursor_line > 0 {
                // join with previous line
                let cur = self.lines[self.cursor_line].clone();
                let prev_len = self.lines[self.cursor_line - 1].chars().count();
                self.lines[self.cursor_line - 1].push_str(&cur);
                self.lines.remove(self.cursor_line);
                self.cursor_line -= 1;
                self.cursor_col = prev_len;
                self.touch();
            }
            return;
        }
        let line = &mut self.lines[self.cursor_line];
        let chars: Vec<char> = line.chars().collect();
        let mut new_line = String::with_capacity(line.len());
        for (i, ch) in chars.iter().enumerate() {
            if i != self.cursor_col - 1 {
                new_line.push(*ch);
            }
        }
        self.cursor_col -= 1;
        *line = new_line;
        self.touch();
    }

    /// Delete char under cursor (delete key).
    pub fn delete(&mut self) {
        self.checkpoint(None);
        self.desired_col = None;
        let max_col = self.line_len(self.cursor_line);
        if self.cursor_col < max_col {
            let line = &mut self.lines[self.cursor_line];
            let chars: Vec<char> = line.chars().collect();
            let mut new_line = String::with_capacity(line.len());
            for (i, ch) in chars.iter().enumerate() {
                if i != self.cursor_col {
                    new_line.push(*ch);
                }
            }
            *line = new_line;
            self.touch();
        } else if self.cursor_line + 1 < self.lines.len() {
            // join next line into current
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.touch();
        }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Get a line's text as a Vec<char> (cheap-ish; used for rendering).
    pub fn line_chars(&self, idx: usize) -> Vec<char> {
        self.lines
            .get(idx)
            .map(|s| s.chars().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_text_roundtrip() {
        let mut b = Buffer::new("hi");
        b.line_end(); // cursor at end
        b.insert_char('!');
        assert_eq!(b.text(), "hi!");
        b.insert_newline();
        assert_eq!(b.text(), "hi!\n");
    }

    #[test]
    fn backspace_joins_lines() {
        let mut b = Buffer::new("ab\ncd");
        b.cursor_line = 1; // start of second line
        b.cursor_col = 0;
        b.backspace();
        assert_eq!(b.text(), "abcd");
        assert_eq!(b.cursor_line, 0);
        assert_eq!(b.cursor_col, 2);
    }

    #[test]
    fn backspace_mid_line() {
        let mut b = Buffer::new("abc");
        b.cursor_col = 2; // between b and c
        b.backspace();
        assert_eq!(b.text(), "ac");
        assert_eq!(b.cursor_col, 1);
    }

    #[test]
    fn delete_under_cursor() {
        let mut b = Buffer::new("abc");
        b.cursor_col = 0;
        b.delete();
        assert_eq!(b.text(), "bc");
    }

    #[test]
    fn delete_at_eol_joins_next() {
        let mut b = Buffer::new("ab\ncd");
        b.cursor_line = 0;
        b.cursor_col = 2;
        b.delete();
        assert_eq!(b.text(), "abcd");
    }

    #[test]
    fn insert_text_multiline() {
        let mut b = Buffer::new("hello world");
        // cursor after "hello " (col 6, at the 'w')
        b.cursor_col = 6;
        b.insert_text("there\nnew line");
        assert_eq!(b.text(), "hello there\nnew lineworld");
        assert_eq!(b.cursor_line, 1);
    }

    #[test]
    fn word_movement_forward() {
        let mut b = Buffer::new("foo bar baz");
        b.word_forward();
        assert_eq!(b.cursor_col, 3, "after first word");
        b.word_forward(); // skip space, land after bar
        assert_eq!(b.cursor_col, 7);
    }

    #[test]
    fn word_movement_back() {
        let mut b = Buffer::new("foo bar baz");
        b.cursor_col = 7; // after "bar"
        b.word_back();
        assert_eq!(b.cursor_col, 4);
    }

    #[test]
    fn up_down_with_desired_col() {
        let mut b = Buffer::new("short\nthis is a longer line\ntiny");
        b.cursor_line = 1;
        b.cursor_col = 10;
        b.up();
        assert_eq!(b.cursor_line, 0);
        assert!(b.cursor_col <= 5); // clamped to short line
        b.down();
        assert_eq!(b.cursor_line, 1);
        assert_eq!(b.cursor_col, 10, "desired col restored");
    }

    #[test]
    fn navigation_at_boundaries() {
        let mut b = Buffer::new("a\nb\nc");
        b.cursor_line = 0; b.cursor_col = 0;
        b.left(); // no-op
        assert_eq!((b.cursor_line, b.cursor_col), (0, 0));
        b.line_end();
        b.right(); // wrap to next line
        assert_eq!((b.cursor_line, b.cursor_col), (1, 0));
    }

    #[test]
    fn empty_buffer_safe() {
        let b = Buffer::new("");
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn smart_list_continues_bullet() {
        let mut b = Buffer::new("- one");
        b.line_end();
        b.insert_newline_smart();
        assert_eq!(b.text(), "- one\n- ");
    }

    #[test]
    fn smart_list_continues_ordered() {
        let mut b = Buffer::new("1. one");
        b.line_end();
        b.insert_newline_smart();
        assert_eq!(b.text(), "1. one\n2. ");
        b.insert_text("two");
        b.line_end();
        b.insert_newline_smart();
        assert_eq!(b.text(), "1. one\n2. two\n3. ");
    }

    #[test]
    fn smart_list_continues_task() {
        let mut b = Buffer::new("- [ ] todo");
        b.line_end();
        b.insert_newline_smart();
        assert_eq!(b.text(), "- [ ] todo\n- [ ] ");
    }

    #[test]
    fn smart_list_empty_item_outdents() {
        let mut b = Buffer::new("- one\n- ");
        b.cursor_line = 1;
        b.line_end();
        b.insert_newline_smart();
        assert_eq!(b.text(), "- one\n\n");
    }

    #[test]
    fn duplicate_line_works() {
        let mut b = Buffer::new("a\nb\nc");
        b.cursor_line = 1;
        b.duplicate_line();
        assert_eq!(b.text(), "a\nb\nb\nc");
        assert_eq!(b.cursor_line, 2);
    }

    #[test]
    fn delete_line_works() {
        let mut b = Buffer::new("a\nb\nc");
        b.cursor_line = 1;
        b.delete_line();
        assert_eq!(b.text(), "a\nc");
        assert_eq!(b.cursor_line, 1);
    }

    #[test]
    fn delete_last_line_safe() {
        let mut b = Buffer::new("only");
        b.delete_line();
        assert_eq!(b.text(), "");
        assert_eq!(b.line_count(), 1);
    }

    #[test]
    fn undo_redo_basic() {
        let mut b = Buffer::new("hello");
        b.line_end();
        b.insert_char('!'); // "hello!"
        b.insert_char('?'); // "hello!?" (coalesced)
        assert_eq!(b.text(), "hello!?");
        b.undo();
        assert_eq!(b.text(), "hello");
        b.redo();
        assert_eq!(b.text(), "hello!?");
    }

    #[test]
    fn undo_backspace() {
        let mut b = Buffer::new("abc");
        b.line_end();
        b.backspace(); // "ab"
        b.backspace(); // "a"
        assert_eq!(b.text(), "a");
        b.undo(); // back to "ab"
        assert_eq!(b.text(), "ab");
        b.undo(); // back to "abc"
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn auto_indent_preserves_whitespace() {
        let mut b = Buffer::new("    indented code");
        b.line_end();
        b.insert_newline_smart();
        assert_eq!(b.text(), "    indented code\n    ");
        assert_eq!(b.cursor_col, 4);
    }

    #[test]
    fn undo_group_then_new_edit_clears_future() {
        let mut b = Buffer::new("x");
        b.line_end();
        b.insert_char('y');
        b.undo();
        assert_eq!(b.text(), "x");
        b.insert_char('z'); // new edit clears redo
        assert_eq!(b.text(), "xz");
        b.redo(); // no-op
        assert_eq!(b.text(), "xz");
    }
}

/// Detect a Markdown list prefix at the start of a line and return both the
/// byte length consumed and the prefix string to use for the next line.
struct Prefix {
    orig_len: usize,
    next: String,
}

fn list_prefix(line: &str) -> Option<Prefix> {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    // leading spaces (indent)
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let indent = i;
    let rest = &line[indent..];
    // task list / bullet
    if let Some(stripped) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")).or_else(|| rest.strip_prefix("+ ")) {
        let marker_len = 2;
        // task list checkbox?
        if stripped.starts_with("[ ] ") {
            return Some(Prefix {
                orig_len: indent + marker_len + 4,
                next: format!("{}{}[ ] ", &line[..indent], &rest[..marker_len]),
            });
        }
        if stripped.starts_with("[x] ") || stripped.starts_with("[X] ") {
            return Some(Prefix {
                orig_len: indent + marker_len + 4,
                next: format!("{}{}[ ] ", &line[..indent], &rest[..marker_len]),
            });
        }
        return Some(Prefix {
            orig_len: indent + marker_len,
            next: format!("{}{}", &line[..indent], &rest[..marker_len]),
        });
    }
    // ordered list
    let digits_end = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_end > 0 && rest[digits_end..].starts_with(". ") {
        let num: u64 = rest[..digits_end].parse().unwrap_or(1);
        let marker = format!("{}. ", num);
        let next_marker = format!("{}. ", num + 1);
        return Some(Prefix {
            orig_len: indent + marker.len(),
            next: format!("{}{}", &line[..indent], next_marker),
        });
    }
    None
}
