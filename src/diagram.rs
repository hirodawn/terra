//! Native terminal diagram renderer for Mermaid flowcharts and a PlantUML subset.
//!
//! Parses the text syntax into a node/edge graph, computes a hierarchical
//! layout, and paints boxes + orthogonal edges with Unicode box-drawing glyphs
//! directly into a ratatui buffer — no SVG, no external tools.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Rect,
    Round,
    Diamond,
    Stadium,
    Circle,
    Cylinder,
}
impl Shape {
    fn from_open(c: char) -> Option<(Shape, char)> {
        // returns (shape, closing char) for the opening bracket
        match c {
            '[' => Some((Shape::Rect, ']')),
            '(' => Some((Shape::Round, ')')),
            '{' => Some((Shape::Diamond, '}')),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct DNode {
    id: String,
    label: String,
    shape: Shape,
    // layout outputs
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Clone, Copy)]
enum Dir {
    TD,
    LR,
}

#[derive(Clone)]
struct DEdge {
    from: usize,
    to: usize,
    label: Option<String>,
    dashed: bool,
    dotted: bool,
    thick: bool,
}

struct Diagram {
    nodes: Vec<DNode>,
    edges: Vec<DEdge>,
    dir: Dir,
}

impl Diagram {
    #[allow(dead_code)]
    fn find_or(&mut self, id: &str) -> usize {
        if let Some(i) = self.nodes.iter().position(|n| n.id == id) {
            return i;
        }
        self.nodes.push(DNode {
            id: id.to_string(),
            label: id.to_string(),
            shape: Shape::Rect,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        });
        self.nodes.len() - 1
    }
}

/// Paint a diagram block. Returns the number of rows consumed.
pub fn render_block(buf: &mut Buffer, area: ratatui::layout::Rect, lang: &str, src: &str) -> usize {
    if lang == "plantuml" || lang == "puml" {
        return match parse_plantuml(src) {
            Some(d) => draw(buf, area, &d),
            None => 0,
        };
    }
    // Mermaid: dispatch on the diagram type declared in the source.
    match detect_kind(src) {
        Kind::Sequence => render_sequence(buf, area, src),
        Kind::State => match parse_state(src) {
            Some(d) => draw(buf, area, &d),
            None => 0,
        },
        Kind::Class => render_class(buf, area, src),
        Kind::Flowchart => match parse_mermaid(src) {
            Some(d) => draw(buf, area, &d),
            None => 0,
        },
    }
}

#[derive(PartialEq, Debug)]
enum Kind {
    Flowchart,
    Sequence,
    State,
    Class,
}

fn detect_kind(src: &str) -> Kind {
    for line in src.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("%%") {
            continue;
        }
        let low = t.to_lowercase();
        if low.starts_with("sequencediagram") {
            return Kind::Sequence;
        }
        if low.starts_with("statediagram") {
            return Kind::State;
        }
        if low.starts_with("classdiagram") {
            return Kind::Class;
        }
        return Kind::Flowchart;
    }
    Kind::Flowchart
}

// ---------------- Mermaid flowchart parser ----------------

fn parse_mermaid(src: &str) -> Option<Diagram> {
    let mut dir = Dir::TD;
    let mut nodes: Vec<DNode> = Vec::new();
    let mut edges: Vec<DEdge> = Vec::new();
    let mut started = false;

    let find_or = |nodes: &mut Vec<DNode>, id: &str| -> usize {
        if let Some(i) = nodes.iter().position(|n| n.id == id) {
            return i;
        }
        nodes.push(DNode { id: id.to_string(), label: id.to_string(), shape: Shape::Rect, x: 0, y: 0, w: 0, h: 0 });
        nodes.len() - 1
    };

    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        // header: graph/flowchart + direction
        let lower = line.to_lowercase();
        if !started {
            if let Some(rest) = lower.strip_prefix("flowchart").or_else(|| lower.strip_prefix("graph")) {
                let d = rest.trim();
                dir = match d {
                    "lr" | "rl" => Dir::LR,
                    _ => Dir::TD,
                };
                started = true;
                continue;
            }
            // no header; treat the whole thing as flowchart TD
            started = true;
        }
        // skip lines we don't handle
        if lower.starts_with("classdef")
            || lower.starts_with("style")
            || lower.starts_with("class ")
            || lower.starts_with("linkstyle")
            || lower.starts_with("subgraph")
            || lower.starts_with("end")
        {
            continue;
        }
        // split statements on ';'
        for stmt in line.split(';') {
            let _ = parse_chain(stmt.trim(), &mut nodes, &mut edges, &find_or);
        }
    }
    Some(Diagram { nodes, edges, dir })
}

/// Parse a chain like `A[Alpha] -->|yes| B --> C{Cond}`.
fn parse_chain(
    s: &str,
    nodes: &mut Vec<DNode>,
    edges: &mut Vec<DEdge>,
    find_or: &dyn Fn(&mut Vec<DNode>, &str) -> usize,
) {
    if s.is_empty() {
        return;
    }
    let bytes: Vec<char> = s.chars().collect();
    let n = bytes.len();
    let mut i = 0usize;

    // first node
    let mut cur = match parse_node_at(&bytes, &mut i, n, nodes, find_or) {
        Some(x) => x,
        None => return,
    };

    loop {
        // skip ws
        while i < n && bytes[i].is_whitespace() {
            i += 1;
        }
        let op = match parse_edge_op(&bytes, &mut i) {
            Some(o) => o,
            None => break,
        };
        while i < n && bytes[i].is_whitespace() {
            i += 1;
        }
        // optional edge label |text| (or -->|text| form)
        let mut label: Option<String> = None;
        if i < n && bytes[i] == '|' {
            i += 1;
            let ls = i;
            while i < n && bytes[i] != '|' {
                i += 1;
            }
            label = Some(bytes[ls..i].iter().collect());
            if i < n {
                i += 1;
            }
            while i < n && bytes[i].is_whitespace() {
                i += 1;
            }
        }
        let target = match parse_node_at(&bytes, &mut i, n, nodes, find_or) {
            Some(x) => x,
            None => break,
        };
        edges.push(DEdge { from: cur, to: target, label, dashed: op.dashed, dotted: op.dotted, thick: op.thick });
        cur = target;
    }
}

/// Parse a node (ident + optional shape bracket) at bytes[*i..], register it,
/// and return its index. Advances *i past the node.
fn parse_node_at(
    bytes: &[char],
    i: &mut usize,
    n: usize,
    nodes: &mut Vec<DNode>,
    find_or: &dyn Fn(&mut Vec<DNode>, &str) -> usize,
) -> Option<usize> {
    while *i < n && bytes[*i].is_whitespace() {
        *i += 1;
    }
    let id_start = *i;
    while *i < n && (bytes[*i].is_alphanumeric() || bytes[*i] == '_' || bytes[*i] == '-') {
        *i += 1;
    }
    if *i == id_start {
        return None;
    }
    let id: String = bytes[id_start..*i].iter().collect();
    let had_shape = *i < n && (bytes[*i] == '[' || bytes[*i] == '(' || bytes[*i] == '{');
    let idx = find_or(nodes, &id);
    if had_shape {
        let (shape, close) = Shape::from_open(bytes[*i]).unwrap();
        let (shape, label, consumed) = read_shape(bytes, *i, shape, close);
        *i += consumed;
        nodes[idx].label = label;
        nodes[idx].shape = shape;
    }
    Some(idx)
}

struct EdgeOp {
    dashed: bool,
    dotted: bool,
    thick: bool,
}

fn parse_edge_op(bytes: &[char], i: &mut usize) -> Option<EdgeOp> {
    let start = *i;
    let n = bytes.len();
    // gather the operator run: chars in {- > < = .} and '|'
    let mut has = String::new();
    while *i < n {
        let c = bytes[*i];
        if c == '-' || c == '>' || c == '<' || c == '=' || c == '.' {
            has.push(c);
            *i += 1;
        } else {
            break;
        }
    }
    if has.is_empty() || *i == start {
        return None;
    }
    let dotted = has.contains('.') && has.contains('-');
    let dashed = has.contains('.') && !dotted;
    let thick = has.contains('=');
    Some(EdgeOp { dashed, dotted, thick })
}

/// Read a shape bracket starting at bytes[i] (which is the opener). Returns
/// (resolved_shape, label, chars_consumed_including_closers).
fn read_shape(bytes: &[char], i: usize, shape: Shape, close: char) -> (Shape, String, usize) {
    let n = bytes.len();
    let o = bytes[i];
    // double-bracket forms
    let two = i + 1 < n && bytes[i + 1] == o;
    let _ = two;
    if o == '(' && i + 1 < n && bytes[i + 1] == '(' {
        // circle ((...))
        let ls = i + 2;
        let mut j = ls;
        while j + 1 < n && !(bytes[j] == ')' && bytes[j + 1] == ')') {
            j += 1;
        }
        let label: String = bytes[ls..j].iter().collect();
        let consumed = (j + 2) - i + 1; // through `))`
        let consumed = consumed.min(n - i);
        return (Shape::Circle, label, consumed);
    }
    if o == '[' && i + 1 < n && bytes[i + 1] == '(' {
        // stadium ([...])
        let ls = i + 2;
        let mut j = ls;
        while j + 1 < n && !(bytes[j] == ']' && bytes[j + 1] == ')') {
            j += 1;
        }
        let label: String = bytes[ls..j].iter().collect();
        let consumed = ((j + 2) - i + 1).min(n - i);
        return (Shape::Stadium, label, consumed);
    }
    if o == '[' && i + 1 < n && bytes[i + 1] == '(' {
        // (covered above)
    }
    if o == '(' && i + 1 < n && bytes[i + 1] == '[' {
        // cylinder [(...)]
        let ls = i + 2;
        let mut j = ls;
        while j + 1 < n && !(bytes[j] == ']' && bytes[j + 1] == ')') {
            j += 1;
        }
        let label: String = bytes[ls..j].iter().collect();
        let consumed = ((j + 2) - i + 1).min(n - i);
        return (Shape::Cylinder, label, consumed);
    }
    // simple single bracket
    let ls = i + 1;
    let mut j = ls;
    while j < n && bytes[j] != close {
        j += 1;
    }
    let label: String = bytes[ls..j].iter().collect();
    let consumed = if j < n { (j + 1) - i } else { j - i };
    let consumed = consumed.min(n - i);
    (shape, label, consumed)
}

// ---------------- Mermaid state diagram (reuses flowchart engine) ----------------

fn parse_state(src: &str) -> Option<Diagram> {
    let mut nodes: Vec<DNode> = Vec::new();
    let mut edges: Vec<DEdge> = Vec::new();
    let find_or = |nodes: &mut Vec<DNode>, id: &str, label: Option<String>, shape: Shape| -> usize {
        if let Some(i) = nodes.iter().position(|n| n.id == id) {
            if let Some(l) = label { nodes[i].label = l; }
            nodes[i].shape = shape;
            return i;
        }
        nodes.push(DNode {
            id: id.to_string(),
            label: label.unwrap_or_else(|| id.to_string()),
            shape,
            x: 0, y: 0, w: 0, h: 0,
        });
        nodes.len() - 1
    };
    let mut started = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") { continue; }
        let low = line.to_lowercase();
        if !started {
            if low.starts_with("statediagram") { started = true; continue; }
            started = true;
        }
        if low.starts_with("note") || low == "end" { continue; }
        // A --> B : label
        if let Some(idx) = find_arrow(line) {
            let (lhs, rest) = line.split_at(idx);
            let op_len = arrow_len(rest);
            let op = &rest[..op_len];
            let after = rest[op_len..].trim();
            let (rhs, label) = match after.split_once(':') {
                Some((b, l)) => (b.trim(), Some(l.trim().to_string())),
                None => (after.trim(), None),
            };
            let lhs = lhs.trim();
            if lhs.is_empty() || rhs.is_empty() { continue; }
            let (fid, fshape, flabel) = state_node(lhs, true);
            let (tid, tshape, tlabel) = state_node(rhs, false);
            let f = find_or(&mut nodes, &fid, flabel, fshape);
            let t = find_or(&mut nodes, &tid, tlabel, tshape);
            let dashed = op.contains('.') || op.contains('-') && op.matches('-').count() > 2;
            edges.push(DEdge { from: f, to: t, label, dashed, dotted: false, thick: false });
        }
    }
    if nodes.is_empty() { return None; }
    Some(Diagram { nodes, edges, dir: Dir::TD })
}

/// Map `[*]` to a start/end circle node; otherwise use the given id.
fn state_node(s: &str, is_source: bool) -> (String, Shape, Option<String>) {
    let t = s.trim();
    if t == "[*]" {
        if is_source {
            ("__start".to_string(), Shape::Circle, Some("start".to_string()))
        } else {
            ("__end".to_string(), Shape::Circle, Some("end".to_string()))
        }
    } else {
        (t.to_string(), Shape::Round, None)
    }
}

fn find_arrow(s: &str) -> Option<usize> {
    // find the first occurrence of an arrow made of - and > (e.g. -->, ->, -.->, ==>)
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '-' || bytes[i] == '=' {
            let start = i;
            while i < bytes.len() && (bytes[i] == '-' || bytes[i] == '=' || bytes[i] == '.' || bytes[i] == '>') {
                i += 1;
            }
            if bytes[start..i].iter().any(|&c| c == '>') {
                return Some(start);
            }
        } else {
            i += 1;
        }
    }
    None
}

fn arrow_len(s: &str) -> usize {
    s.chars().take_while(|&c| c == '-' || c == '=' || c == '.' || c == '>').count()
}

// ---------------- Mermaid class diagram (subset) ----------------

struct ClsClass { name: String, members: Vec<String> }
struct ClsRel { from: String, to: String, label: String }

fn parse_class(src: &str) -> (Vec<ClsClass>, Vec<ClsRel>) {
    let mut classes: Vec<ClsClass> = Vec::new();
    let mut rels: Vec<ClsRel> = Vec::new();
    let find_or = |classes: &mut Vec<ClsClass>, name: &str| -> usize {
        if let Some(i) = classes.iter().position(|c| c.name == name) { return i; }
        classes.push(ClsClass { name: name.to_string(), members: Vec::new() });
        classes.len() - 1
    };
    let mut started = false;
    let mut cur: Option<usize> = None;
    let mut in_body = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() { continue; }
        let low = line.to_lowercase();
        if !started {
            if low.starts_with("classdiagram") { started = true; continue; }
            started = true;
        }
        if in_body {
            if line == "}" || line.starts_with('}') { in_body = false; cur = None; continue; }
            if let Some(i) = cur { classes[i].members.push(line.to_string()); }
            continue;
        }
        if let Some(rest) = line.strip_prefix("class ").or_else(|| line.strip_prefix("class\t")) {
            let name = rest.trim().trim_end_matches('{').trim();
            if !name.is_empty() && !name.contains(' ') {
                let i = find_or(&mut classes, name);
                if rest.ends_with('{') { in_body = true; cur = Some(i); }
            }
            continue;
        }
        // relationship: A <|-- B : label
        for token in ["<|--", "--|>", "*--", "--*", "o--", "--o", "-->", "<--", "--", ".."] {
            if let Some((a, b)) = line.split_once(token) {
                let (b, label) = match b.split_once(':') {
                    Some((x, l)) => (x.trim(), l.trim().to_string()),
                    None => (b.trim(), String::new()),
                };
                let a = a.trim();
                if !a.is_empty() && !b.is_empty() {
                    find_or(&mut classes, a);
                    find_or(&mut classes, b);
                    rels.push(ClsRel { from: a.to_string(), to: b.to_string(), label });
                }
                break;
            }
        }
    }
    (classes, rels)
}

fn render_class(buf: &mut Buffer, area: ratatui::layout::Rect, src: &str) -> usize {
    let (classes, rels) = parse_class(src);
    if classes.is_empty() { return 0; }
    let accent = Color::Rgb(121, 192, 255);
    let fg = Color::Rgb(220, 225, 235);
    let bg = Color::Rgb(28, 34, 47);
    let border = Style::default().fg(accent).bg(bg);
    let body_style = Style::default().fg(fg).bg(bg);
    let x0 = area.x as i32 + 4;
    let y0 = area.y as i32 + 1;
    let col_w = 26i32;
    let per_row = (area.width as i32 / col_w).max(1);
    let put = |buf: &mut Buffer, x: i32, y: i32, ch: char, s: Style| {
        if x < 0 || y < 0 || x >= buf.area.width as i32 || y >= buf.area.height as i32 { return; }
        let c = &mut buf[(x as u16, y as u16)]; c.set_char(ch); c.set_style(s);
    };
    let mut positions: std::collections::HashMap<String, (i32, i32, i32)> = std::collections::HashMap::new();
    let mut max_bottom = y0;
    for (i, cls) in classes.iter().enumerate() {
        let col = (i as i32) % per_row;
        let row = (i as i32) / per_row;
        let cx = x0 + col * col_w;
        let cy = y0 + row * (10); // rough row stride; refined below
        // width from longest member/name
        let inner = cls.members.iter().map(|m| m.chars().count()).chain(std::iter::once(cls.name.chars().count())).max().unwrap_or(3).max(3) as i32;
        let w = inner + 4;
        let _h = (2 + cls.members.len() as i32).max(3);
        // box
        put(buf, cx, cy, '┌', border);
        put(buf, cx + w - 1, cy, '┐', border);
        for k in 1..(w - 1) { put(buf, cx + k, cy, '─', border); }
        // name row (centered)
        put(buf, cx, cy + 1, '│', border);
        put(buf, cx + w - 1, cy + 1, '│', border);
        let pad = (inner - cls.name.chars().count() as i32).max(0) / 2;
        for (k, c) in cls.name.chars().enumerate() {
            put(buf, cx + 2 + pad + k as i32, cy + 1, c, body_style.add_modifier(Modifier::BOLD));
        }
        for k in 2..(w - 1) { put(buf, cx + k, cy + 1, ' ', body_style); }
        // separator
        if !cls.members.is_empty() {
            put(buf, cx, cy + 2, '├', border);
            put(buf, cx + w - 1, cy + 2, '┤', border);
            for k in 1..(w - 1) { put(buf, cx + k, cy + 2, '─', border); }
        }
        // members
        for (mi, m) in cls.members.iter().enumerate() {
            let ry = cy + 3 + mi as i32;
            put(buf, cx, ry, '│', border);
            put(buf, cx + w - 1, ry, '│', border);
            for (k, c) in m.chars().enumerate() {
                if k + 2 < (w - 1) as usize { put(buf, cx + 2 + k as i32, ry, c, body_style); }
            }
            for k in (m.chars().count() as i32 + 2)..(w - 1) { put(buf, cx + k, ry, ' ', body_style); }
        }
        let bottom = cy + 3 + cls.members.len() as i32;
        put(buf, cx, bottom, '└', border);
        put(buf, cx + w - 1, bottom, '┘', border);
        for k in 1..(w - 1) { put(buf, cx + k, bottom, '─', border); }
        positions.insert(cls.name.clone(), (cx + w / 2, cy + 1, cx + w));
        max_bottom = max_bottom.max(bottom);
    }
    // relationships as simple labeled arrows between class centers (best-effort)
    let edge_col = Color::Rgb(139, 148, 158);
    let mut ry = max_bottom + 2;
    for rel in &rels {
        if ry >= buf.area.height as i32 - 1 { break; }
        let label = if rel.label.is_empty() {
            format!("{} ──▶ {}", rel.from, rel.to)
        } else {
            format!("{} ──▶ {} : {}", rel.from, rel.to, rel.label)
        };
        for (k, c) in label.chars().enumerate() {
            put(buf, x0 + k as i32, ry, c, Style::default().fg(edge_col));
        }
        ry += 1;
    }
    let _ = positions;
    ((ry - y0) as usize).max(1)
}

// ---------------- Mermaid sequence diagram ----------------

struct SeqParticipant { name: String }
enum SeqEvent {
    Msg { from: usize, to: usize, label: String, dashed: bool },
    Note { over: Vec<usize>, text: String },
}
struct Sequence { participants: Vec<SeqParticipant>, events: Vec<SeqEvent> }

fn parse_sequence(src: &str) -> Option<Sequence> {
    let mut participants: Vec<SeqParticipant> = Vec::new();
    let mut events: Vec<SeqEvent> = Vec::new();
    let mut started = false;
    let find_p = |participants: &mut Vec<SeqParticipant>, name: &str| -> usize {
        if let Some(i) = participants.iter().position(|p| p.name == name) { return i; }
        participants.push(SeqParticipant { name: name.to_string() });
        participants.len() - 1
    };
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") { continue; }
        let low = line.to_lowercase();
        if !started {
            if low.starts_with("sequencediagram") { started = true; continue; }
            started = true;
        }
        if low.starts_with("autonumber") || low.starts_with("activate") || low.starts_with("deactivate") {
            continue;
        }
        if let Some(_rest) = low.strip_prefix("participant") {
            let name = line["participant".len()..].trim();
            let name = match name.split_once(" as ") { Some((a, _)) => a.trim(), None => name };
            if !name.is_empty() { find_p(&mut participants, name); }
            continue;
        }
        if low.starts_with("note") {
            // Note over A,B: text   /  Note over A: text
            let after = line["note".len()..].trim();
            let (targets, text) = match after.split_once(':') {
                Some((a, t)) => (a.trim(), t.trim()),
                None => (after, ""),
            };
            let over = targets.strip_prefix("over").or_else(|| targets.strip_prefix("left of")).or_else(|| targets.strip_prefix("right of")).unwrap_or(targets).trim();
            let mut idxs = Vec::new();
            for n in over.split(',') {
                let n = n.trim();
                if !n.is_empty() { idxs.push(find_p(&mut participants, n)); }
            }
            if idxs.is_empty() && !participants.is_empty() { idxs.push(0); }
            events.push(SeqEvent::Note { over: idxs, text: text.to_string() });
            continue;
        }
        // message: A->>B: text  / A-->>B: text / A->B / A--xB etc.
        if let Some(m) = parse_seq_msg(line, &mut participants, &find_p) {
            events.push(m);
        }
    }
    if participants.is_empty() { return None; }
    Some(Sequence { participants, events })
}

fn parse_seq_msg(line: &str, participants: &mut Vec<SeqParticipant>, find_p: &dyn Fn(&mut Vec<SeqParticipant>, &str) -> usize) -> Option<SeqEvent> {
    // find arrow token
    let bytes: Vec<char> = line.chars().collect();
    let mut start = None;
    let mut end = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '-' {
            if start.is_none() { start = Some(i); }
            while i < bytes.len() && (bytes[i] == '-' || bytes[i] == '.' || bytes[i] == '>' || bytes[i] == 'x' || bytes[i] == ')') {
                i += 1;
            }
            end = Some(i);
            break;
        }
        i += 1;
    }
    let (s, e) = (start?, end?);
    let lhs: String = bytes[..s].iter().collect();
    let op: String = bytes[s..e].iter().collect();
    let after: String = bytes[e..].iter().collect();
    let (rhs, label) = match after.split_once(':') {
        Some((b, l)) => (b.trim(), l.trim().to_string()),
        None => (after.trim(), String::new()),
    };
    let lhs = lhs.trim();
    if lhs.is_empty() || rhs.is_empty() { return None; }
    let from = find_p(participants, lhs);
    let to = find_p(participants, rhs);
    let dashed = op.contains('.') || op.matches('-').count() >= 3;
    Some(SeqEvent::Msg { from, to, label, dashed })
}

fn render_sequence(buf: &mut Buffer, area: ratatui::layout::Rect, src: &str) -> usize {
    let seq = match parse_sequence(src) { Some(s) => s, None => return 0 };
    let n = seq.participants.len();
    if n == 0 { return 0; }
    let accent = Color::Rgb(121, 192, 255);
    let fg = Color::Rgb(220, 225, 235);
    let bg = Color::Rgb(28, 34, 47);
    let border = Style::default().fg(accent).bg(bg);
    let life = Color::Rgb(90, 99, 116);
    let label_col = Color::Rgb(200, 210, 225);
    let edge = Color::Rgb(139, 148, 158);
    let x0 = area.x as i32 + 2;
    let avail = (area.width as i32 - 4).max(n as i32 * 6);
    // column x for each participant
    let col_x: Vec<i32> = (0..n).map(|i| x0 + ((i as i32 + 1) * avail) / (n as i32 + 1)).collect();
    let put = |buf: &mut Buffer, x: i32, y: i32, ch: char, s: Style| {
        if x < 0 || y < 0 || x >= buf.area.width as i32 || y >= buf.area.height as i32 { return; }
        let c = &mut buf[(x as u16, y as u16)]; c.set_char(ch); c.set_style(s);
    };
    let write = |buf: &mut Buffer, x: i32, y: i32, s: &str, col: Style| {
        for (k, ch) in s.chars().enumerate() { put(buf, x + k as i32, y, ch, col); }
    };
    let mut y = area.y as i32 + 1;
    // header boxes
    let mut maxw = 6i32;
    for (i, p) in seq.participants.iter().enumerate() {
        let w = (p.name.chars().count() as i32 + 4).max(6);
        maxw = maxw.max(w);
        let cx = col_x[i] - w / 2;
        put(buf, cx, y, '╭', border); put(buf, cx + w - 1, y, '╮', border);
        put(buf, cx, y + 2, '╰', border); put(buf, cx + w - 1, y + 2, '╯', border);
        for k in 1..(w - 1) { put(buf, cx + k, y, '─', border); put(buf, cx + k, y + 2, '─', border); }
        put(buf, cx, y + 1, '│', border); put(buf, cx + w - 1, y + 1, '│', border);
        let pad = (w - 2 - p.name.chars().count() as i32).max(0) / 2;
        for (k, ch) in p.name.chars().enumerate() { put(buf, cx + 1 + pad + k as i32, y + 1, ch, Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)); }
        for k in (1 + pad + p.name.chars().count() as i32)..(w - 1) { put(buf, cx + k, y + 1, ' ', Style::default().bg(bg)); }
    }
    y += 3;
    let top = y;
    // events
    for ev in &seq.events {
        match ev {
            SeqEvent::Msg { from, to, label, dashed } => {
                let fx = col_x[*from];
                let tx = col_x[*to];
                let hch = if *dashed { '┄' } else { '─' };
                // label row
                if !label.is_empty() {
                    let mid = (fx + tx) / 2 - (label.chars().count() as i32) / 2;
                    write(buf, mid, y, label, Style::default().fg(label_col).bg(bg));
                    y += 1;
                }
                let (lo, hi) = if fx <= tx { (fx, tx) } else { (tx, fx) };
                for x in (lo + 1)..hi { put(buf, x, y, hch, Style::default().fg(edge).bg(bg)); }
                put(buf, lo, y, '●', Style::default().fg(edge).bg(bg));
                // arrowhead
                let head = if fx <= tx { '▶' } else { '◀' };
                put(buf, hi, y, head, Style::default().fg(accent).bg(bg));
                y += 2;
            }
            SeqEvent::Note { over, text } => {
                let lo = over.iter().map(|&i| col_x[i]).min().unwrap_or(x0);
                let hi = over.iter().map(|&i| col_x[i] + maxw).max().unwrap_or(x0 + maxw);
                let w = (hi - lo).max(text.chars().count() as i32 + 4);
                put(buf, lo, y, '┌', border); put(buf, lo + w - 1, y, '┐', border);
                put(buf, lo, y + 1, '│', border); put(buf, lo + w - 1, y + 1, '│', border);
                put(buf, lo, y + 2, '└', border); put(buf, lo + w - 1, y + 2, '┘', border);
                for k in 1..(w - 1) { put(buf, lo + k, y, '─', border); put(buf, lo + k, y + 2, '─', border); }
                for (k, ch) in text.chars().enumerate() { put(buf, lo + 1 + k as i32, y + 1, ch, Style::default().fg(Color::Rgb(210,168,255)).bg(bg)); }
                for k in (1 + text.chars().count() as i32)..(w - 1) { put(buf, lo + k, y + 1, ' ', Style::default().bg(bg)); }
                y += 4;
            }
        }
    }
    // lifelines
    for i in 0..n {
        for ly in top..y {
            put(buf, col_x[i], ly, '┊', Style::default().fg(life).bg(bg));
        }
    }
    ((y - area.y as i32) as usize).max(1)
}

// ---------------- PlantUML (subset) parser ----------------

fn parse_plantuml(src: &str) -> Option<Diagram> {
    let mut nodes: Vec<DNode> = Vec::new();
    let mut edges: Vec<DEdge> = Vec::new();
    let mut prev: Option<usize> = None;

    let find_or = |nodes: &mut Vec<DNode>, id: &str, label: Option<String>| -> usize {
        if let Some(i) = nodes.iter().position(|n| n.id == id) {
            if let Some(l) = label {
                nodes[i].label = l;
            }
            return i;
        }
        nodes.push(DNode {
            id: id.to_string(),
            label: label.unwrap_or_else(|| id.to_string()),
            shape: Shape::Round,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        });
        nodes.len() - 1
    };

    let mut auto = 0usize;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("'") {
            continue;
        }
        if line == "@startuml" || line == "@enduml" {
            continue;
        }
        // sequence-style: A -> B : message
        if let Some(rest) = line.split_once("->") {
            let lhs = rest.0.trim();
            let (rhs, msg) = match rest.1.split_once(':') {
                Some((b, m)) => (b.trim(), Some(m.trim())),
                None => (rest.1.trim(), None),
            };
            if !lhs.is_empty() && !rhs.is_empty()
                && lhs.chars().next().map_or(false, |c| c.is_alphanumeric())
            {
                let f = find_or(&mut nodes, lhs, None);
                let t = find_or(&mut nodes, rhs, None);
                edges.push(DEdge { from: f, to: t, label: msg.map(|s| s.to_string()), dashed: false, dotted: false, thick: false });
                prev = Some(t);
                continue;
            }
        }
        // activity: :action;
        if let Some(rest) = line.strip_prefix(':') {
            let label = rest.trim_end_matches(';').trim().to_string();
            let id = format!("act{}", auto);
            auto += 1;
            let idx = find_or(&mut nodes, &id, Some(label));
            if let Some(p) = prev {
                edges.push(DEdge { from: p, to: idx, label: None, dashed: false, dotted: false, thick: false });
            }
            prev = Some(idx);
            continue;
        }
        if line == "start" || line == "stop" || line == "end" {
            let id = line.to_string();
            let idx = find_or(&mut nodes, &id, Some(line.to_string()));
            if let Some(p) = prev {
                edges.push(DEdge { from: p, to: idx, label: None, dashed: false, dotted: false, thick: false });
            }
            prev = Some(idx);
            continue;
        }
    }
    if nodes.is_empty() {
        return None;
    }
    Some(Diagram { nodes, edges, dir: Dir::TD })
}

// ---------------- Layout ----------------

fn layout(d: &mut Diagram, area_w: i32) {
    let n = d.nodes.len();
    if n == 0 {
        return;
    }
    // size each node
    for node in d.nodes.iter_mut() {
        let label_w = node.label.chars().count() as i32;
        node.w = (label_w + 4).max(5);
        node.h = match node.shape {
            Shape::Diamond => 3,
            _ => 3,
        };
    }
    // layering via longest path from roots (in-degree 0)
    let mut indeg = vec![0usize; n];
    for e in &d.edges {
        indeg[e.to] += 1;
    }
    let mut layer = vec![0i32; n];
    // Kahn-ish: process in topological passes
    let mut changed = true;
    let mut passes = 0usize;
    while changed && passes <= n + 1 {
        changed = false;
        passes += 1;
        for e in &d.edges {
            if layer[e.to] < layer[e.from].saturating_add(1) {
                layer[e.to] = layer[e.from].saturating_add(1);
                changed = true;
            }
        }
    }
    let max_layer = layer.iter().copied().max().unwrap_or(0);
    let _ = max_layer;
    // cap layers (handles cycles so the layout stays compact)
    let cap = (n.saturating_sub(1)) as i32;
    for l in layer.iter_mut() {
        *l = (*l).min(cap);
    }
    let max_layer = layer.iter().copied().max().unwrap_or(0);
    let nlayers = (max_layer + 1) as usize;
    // bucket nodes per layer preserving order
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); nlayers];
    for (idx, l) in layer.iter().enumerate() {
        buckets[*l as usize].push(idx);
    }
    // positions: non-overlapping grid (col = order within layer), then center the
    // whole drawing horizontally.
    let gap_n = 4i32;
    let gap_l = 4i32;
    let max_node_w = d.nodes.iter().map(|nd| nd.w).max().unwrap_or(1);
    let max_node_h = d.nodes.iter().map(|nd| nd.h).max().unwrap_or(1);
    let col_w = max_node_w + gap_n;
    let row_h = max_node_h + gap_l;
    // figure out max columns per layer to size the grid
    let max_cols = buckets.iter().map(|b| b.len()).max().unwrap_or(1).max(1);
    match d.dir {
        Dir::TD => {
            let total_w = col_w * max_cols as i32;
            let x_off = ((area_w - total_w) / 2).max(0);
            for (li, b) in buckets.iter().enumerate() {
                for (ci, &i) in b.iter().enumerate() {
                    d.nodes[i].x = x_off + (ci as i32) * col_w;
                    d.nodes[i].y = (li as i32) * row_h;
                }
            }
        }
        Dir::LR => {
            for (li, b) in buckets.iter().enumerate() {
                for (ci, &i) in b.iter().enumerate() {
                    d.nodes[i].x = (li as i32) * col_w;
                    d.nodes[i].y = (ci as i32) * row_h;
                }
            }
        }
    }
    let _ = max_node_h;
}

// ---------------- Draw ----------------

fn draw(buf: &mut Buffer, area: ratatui::layout::Rect, d: &Diagram) -> usize {
    let mut d = Diagram { nodes: d.nodes.clone(), edges: d.edges.clone(), dir: d.dir };
    let area_w = area.width as i32 - 4;
    layout(&mut d, area_w.max(20));

    // compute bounds
    let mut max_y = 0i32;
    for n in &d.nodes {
        max_y = max_y.max(n.y + n.h);
    }
    let height = (max_y + 2) as usize;

    let accent = Color::Rgb(255, 166, 87);
    let node_fg = Color::Rgb(220, 225, 235);
    let node_bg = Color::Rgb(28, 34, 47);
    let edge_col = Color::Rgb(139, 148, 158);
    let label_col = Color::Rgb(200, 210, 225);
    let x0 = area.x as i32 + 2;
    let y0 = area.y as i32 + 1;

    let put = |buf: &mut Buffer, x: i32, y: i32, ch: char, s: Style| {
        if x < 0 || y < 0 || x >= buf.area.width as i32 || y >= buf.area.height as i32 {
            return;
        }
        let cell = &mut buf[(x as u16, y as u16)];
        cell.set_char(ch);
        cell.set_style(s);
    };

    // draw edges first (so nodes overlap endpoints)
    for e in &d.edges {
        let a = &d.nodes[e.from];
        let b = &d.nodes[e.to];
        let style = Style::default().fg(if e.thick { accent } else { edge_col });
        let dash = if e.dotted { Some('.') } else if e.dashed { Some('-') } else { None };
        // centers of connecting sides
        let (sx, sy, tx, ty, arr) = match d.dir {
            Dir::TD => {
                let sx = a.x + a.w / 2;
                let sy = a.y + a.h; // bottom
                let tx = b.x + b.w / 2;
                let ty = b.y; // top
                (sx, sy, tx, ty, '▼')
            }
            Dir::LR => {
                let sx = a.x + a.w; // right
                let sy = a.y + a.h / 2;
                let tx = b.x; // left
                let ty = b.y + b.h / 2;
                (sx, sy, tx, ty, '▶')
            }
        };
        let mid = match d.dir {
            Dir::TD => (sy + ty) / 2,
            Dir::LR => (sx + tx) / 2,
        };
        let hch = dash.unwrap_or('─');
        let vch = dash.unwrap_or('│');
        match d.dir {
            Dir::TD => {
                // down from sy+1..mid
                for yy in (sy + 1)..=mid {
                    put(buf, x0 + sx, y0 + yy, vch, style);
                }
                if sx != tx {
                    // horizontal across at mid
                    let (lo, hi) = if sx < tx { (sx, tx) } else { (tx, sx) };
                    for xx in lo..=hi {
                        put(buf, x0 + xx, y0 + mid, hch, style);
                    }
                    // corners
                    put(buf, x0 + sx, y0 + mid, if sx < tx { '┐' } else { '┌' }, style);
                    put(buf, x0 + tx, y0 + mid, if sx < tx { '└' } else { '┘' }, style);
                    for yy in (mid + 1)..ty {
                        put(buf, x0 + tx, y0 + yy, vch, style);
                    }
                } else {
                    for yy in (mid + 1)..ty {
                        put(buf, x0 + tx, y0 + yy, vch, style);
                    }
                }
                put(buf, x0 + tx, y0 + ty, arr, style.fg(accent));
            }
            Dir::LR => {
                for xx in (sx + 1)..=mid {
                    put(buf, x0 + xx, y0 + sy, hch, style);
                }
                if sy != ty {
                    let (lo, hi) = if sy < ty { (sy, ty) } else { (ty, sy) };
                    for yy in lo..=hi {
                        put(buf, x0 + mid, y0 + yy, vch, style);
                    }
                    put(buf, x0 + mid, y0 + sy, if sy < ty { '┘' } else { '┐' }, style);
                    put(buf, x0 + mid, y0 + ty, if sy < ty { '┌' } else { '┘' }, style);
                    for xx in (mid + 1)..tx {
                        put(buf, x0 + xx, y0 + ty, hch, style);
                    }
                } else {
                    for xx in (mid + 1)..tx {
                        put(buf, x0 + xx, y0 + ty, hch, style);
                    }
                }
                put(buf, x0 + tx, y0 + ty, arr, style.fg(accent));
            }
        }
        // edge label
        if let Some(lab) = &e.label {
            let lx = match d.dir {
                Dir::TD => x0 + (sx + tx) / 2 - (lab.chars().count() as i32) / 2,
                Dir::LR => x0 + mid + 1,
            };
            let ly = match d.dir {
                Dir::TD => y0 + mid - 1,
                Dir::LR => y0 + (sy + ty) / 2,
            };
            for (k, c) in lab.chars().enumerate() {
                put(buf, lx + k as i32, ly, c, Style::default().fg(label_col));
            }
        }
    }

    // draw nodes
    for node in &d.nodes {
        draw_node(buf, x0 + node.x, y0 + node.y, node, node_fg, node_bg, accent);
    }

    height
}

fn draw_node(
    buf: &mut Buffer,
    x: i32,
    y: i32,
    node: &DNode,
    fg: Color,
    bg: Color,
    accent: Color,
) {
    let w = node.w;
    let inner = (w - 2).max(1);
    let style = Style::default().fg(fg).bg(bg);
    let border = Style::default().fg(accent).bg(bg);
    let put = |buf: &mut Buffer, xx: i32, yy: i32, ch: char, s: Style| {
        if xx < 0 || yy < 0 || xx >= buf.area.width as i32 || yy >= buf.area.height as i32 {
            return;
        }
        let cell = &mut buf[(xx as u16, yy as u16)];
        cell.set_char(ch);
        cell.set_style(s);
    };
    match node.shape {
        Shape::Rect | Shape::Cylinder => {
            put(buf, x, y, '┌', border);
            put(buf, x + w - 1, y, '┐', border);
            put(buf, x, y + 2, '└', border);
            put(buf, x + w - 1, y + 2, '┘', border);
            for i in 1..(w - 1) {
                put(buf, x + i, y, '─', border);
                put(buf, x + i, y + 2, '─', border);
            }
            // label row
            put(buf, x, y + 1, '│', border);
            put(buf, x + w - 1, y + 1, '│', border);
            write_label(buf, x + 1, y + 1, &node.label, inner, style);
            if node.shape == Shape::Cylinder {
                put(buf, x, y, '╭', border);
                put(buf, x + w - 1, y, '╮', border);
            }
        }
        Shape::Round | Shape::Stadium | Shape::Circle => {
            put(buf, x, y, '╭', border);
            put(buf, x + w - 1, y, '╮', border);
            put(buf, x, y + 2, '╰', border);
            put(buf, x + w - 1, y + 2, '╯', border);
            for i in 1..(w - 1) {
                put(buf, x + i, y, '─', border);
                put(buf, x + i, y + 2, '─', border);
            }
            put(buf, x, y + 1, '│', border);
            put(buf, x + w - 1, y + 1, '│', border);
            write_label(buf, x + 1, y + 1, &node.label, inner, style);
        }
        Shape::Diamond => {
            // ╱─────╲  / │label│ \ /─────╲  ... 3-row form
            let top = y;
            let mid = y + 1;
            let bot = y + 2;
            put(buf, x + 1, top, '╱', border);
            put(buf, x + w - 2, top, '╲', border);
            for i in 2..(w - 2) {
                put(buf, x + i, top, '─', border);
            }
            put(buf, x, mid, '│', border);
            put(buf, x + w - 1, mid, '│', border);
            write_label(buf, x + 1, mid, &node.label, inner, style);
            put(buf, x + 1, bot, '╲', border);
            put(buf, x + w - 2, bot, '╱', border);
            for i in 2..(w - 2) {
                put(buf, x + i, bot, '─', border);
            }
        }
    }
    // bold the label a touch
    let _ = (fg, Style::default().add_modifier(Modifier::BOLD));
}

fn write_label(buf: &mut Buffer, x: i32, y: i32, label: &str, inner: i32, style: Style) {
    let chars: Vec<char> = label.chars().take(inner.max(0) as usize).collect();
    let pad = (inner - chars.len() as i32).max(0) / 2;
    for (k, c) in chars.iter().enumerate() {
        let xx = x + pad + k as i32;
        if xx < 0 || y < 0 || xx >= buf.area.width as i32 || y >= buf.area.height as i32 {
            continue;
        }
        let cell = &mut buf[(xx as u16, y as u16)];
        cell.set_char(*c);
        cell.set_style(style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_chain_parses_all_edges() {
        let d = parse_mermaid("graph LR\nA[Start] --> B[Process] --> C[End]").unwrap();
        assert_eq!(d.nodes.len(), 3, "3 nodes");
        assert_eq!(d.edges.len(), 2, "2 edges (chain)");
        assert_eq!(d.nodes[0].label, "Start");
        assert_eq!(d.nodes[2].label, "End");
    }

    #[test]
    fn mermaid_shapes_and_labels() {
        let d = parse_mermaid("graph TD\nA[Box] --> B{Choice}\nB --> C(Round)\nB --> D((Circle))").unwrap();
        let labels: Vec<&str> = d.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(labels.contains(&"Box"));
        assert!(labels.contains(&"Choice"));
        assert!(labels.contains(&"Circle"));
        assert_eq!(d.edges.len(), 3);
    }

    #[test]
    fn mermaid_edge_label() {
        let d = parse_mermaid("graph TD\nA -->|yes| B").unwrap();
        assert_eq!(d.edges.len(), 1);
        assert_eq!(d.edges[0].label.as_deref(), Some("yes"));
    }

    #[test]
    fn plantuml_sequence_parses() {
        let d = parse_plantuml("@startuml\nBob -> Alice : hello\nAlice -> Bob : hi\n@enduml").unwrap();
        assert_eq!(d.nodes.len(), 2, "Bob and Alice");
        assert_eq!(d.edges.len(), 2, "two messages");
        assert_eq!(d.edges[0].label.as_deref(), Some("hello"));
    }

    #[test]
    fn plantuml_activity_parses() {
        let d = parse_plantuml(":first step;\n:second step;").unwrap();
        assert_eq!(d.nodes.len(), 2);
        assert_eq!(d.edges.len(), 1);
    }

    #[test]
    fn render_block_smoke_mermaid() {
        let area = ratatui::layout::Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        let h = render_block(&mut buf, area, "mermaid", "graph LR\nA --> B --> C");
        assert!(h > 0, "diagram should occupy rows");
        let mut found = false;
        for y in 0..area.height {
            for x in 0..area.width {
                let s = buf[(x, y)].symbol();
                if s == "┌" || s == "─" || s == "│" { found = true; }
            }
        }
        assert!(found, "box drawing should appear");
    }

    #[test]
    fn render_block_smoke_plantuml() {
        let area = ratatui::layout::Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        let h = render_block(&mut buf, area, "plantuml", "Bob -> Alice : hello");
        assert!(h > 0);
        let mut has_box = false;
        for y in 0..area.height {
            for x in 0..area.width {
                let s = buf[(x, y)].symbol();
                if s == "╭" || s == "│" || s == "─" { has_box = true; }
            }
        }
        assert!(has_box, "plantuml should render boxes");
    }

    #[test]
    fn sequence_diagram_parses_and_renders() {
        let src = "sequenceDiagram\nparticipant Alice\nparticipant Bob\nAlice->>Bob: Hello\nBob-->>Alice: Hi";
        let s = parse_sequence(src).unwrap();
        assert_eq!(s.participants.len(), 2);
        assert_eq!(s.events.len(), 2);
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let h = render_block(&mut buf, area, "mermaid", src);
        assert!(h > 4);
        // a lifeline glyph or participant box should appear
        let mut found = false;
        for y in 0..area.height { for x in 0..area.width {
            let s = buf[(x,y)].symbol(); if s=="┊"||s=="╭"||s=="▶" { found=true; }
        }}
        assert!(found, "sequence lifelines/boxes should render");
    }

    #[test]
    fn state_diagram_parses() {
        let d = parse_state("stateDiagram-v2\n[*] --> Idle\nIdle --> Done: finish\nDone --> [*]").unwrap();
        assert!(d.nodes.len() >= 3); // start, idle, done, end
        assert!(d.edges.len() >= 3);
        // start/end special nodes exist
        let ids: Vec<&str> = d.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"__start"));
        assert!(ids.contains(&"__end"));
    }

    #[test]
    fn class_diagram_parses() {
        let src = "classDiagram\nclass Animal {\n+String name\n+eat()\n}\nclass Dog {\n+bark()\n}\nAnimal <|-- Dog";
        let (classes, rels) = parse_class(src);
        assert_eq!(classes.len(), 2);
        assert!(classes.iter().any(|c| c.name == "Animal" && c.members.len() == 2));
        assert!(classes.iter().any(|c| c.name == "Dog"));
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].from, "Animal");
        assert_eq!(rels[0].to, "Dog");
    }

    #[test]
    fn detect_kind_dispatches() {
        assert_eq!(detect_kind("sequenceDiagram\nA->>B:x"), Kind::Sequence);
        assert_eq!(detect_kind("stateDiagram-v2\n[*] --> A"), Kind::State);
        assert_eq!(detect_kind("classDiagram\nclass A"), Kind::Class);
        assert_eq!(detect_kind("graph TD\nA-->B"), Kind::Flowchart);
        assert_eq!(detect_kind("flowchart LR\nA-->B"), Kind::Flowchart);
    }
}
