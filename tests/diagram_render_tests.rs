//! Diagram rendering golden tests — complex Mermaid diagrams with expected output.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use terra::diagram::render_block;

fn render(md: &str, lang: &str) -> (Buffer, usize) {
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let h = render_block(&mut buf, area, lang, md);
    (buf, h)
}

fn buf_text(buf: &Buffer, area: Rect) -> String {
    let mut s = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let sym = buf[(x, y)].symbol();
            if sym != " " { s.push_str(sym); }
        }
        s.push('\n');
    }
    s
}

// ===== Linear chain (LR) =====
#[test]
fn flowchart_linear_chain() {
    let (buf, h) = render("graph LR\nA[Start] --> B[Middle] --> C[End]", "mermaid");
    assert!(h > 0);
    let text = buf_text(&buf, Rect::new(0, 0, 80, 10));
    assert!(text.contains("Start"));
    assert!(text.contains("Middle"));
    assert!(text.contains("End"));
    assert!(text.contains('─'), "horizontal edge missing");
    assert!(text.contains('▶'), "arrowhead missing");
}

// ===== Branching =====
#[test]
fn flowchart_branch() {
    let (buf, _) = render("graph TD\nA[Root] --> B[Left]\nA --> C[Right]", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 15));
    assert!(text.contains("Root"));
    assert!(text.contains("Left"));
    assert!(text.contains("Right"));
    assert!(text.contains('▼'), "downward arrowhead");
}

// ===== Diamond / shapes =====
#[test]
fn flowchart_shapes() {
    let (buf, _) = render("graph TD\nA[Rect] --> B{Diamond}\nB --> C(Round)\nB --> D((Circle))", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 20));
    for lbl in ["Rect", "Diamond", "Round", "Circle"] {
        assert!(text.contains(lbl), "{} missing", lbl);
    }
    assert!(text.contains('╱') || text.contains('╲'), "diamond diagonals");
}

// ===== Edge labels =====
#[test]
fn flowchart_edge_labels() {
    let (buf, _) = render("graph TD\nA -->|yes| B[Yes]\nA -->|no| C[No]", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 20));
    assert!(text.contains("yes"));
    assert!(text.contains("no"));
}

// ===== Dashed edge =====
#[test]
fn flowchart_dashed() {
    let (buf, _) = render("graph TD\nA -.-> B[Dashed]\nA --> C[Solid]", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 20));
    assert!(text.contains("Dashed"));
    assert!(text.contains("Solid"));
}

// ===== Colors =====
#[test]
fn flowchart_colors() {
    let (buf, _) = render("graph TD\nA[Red] --> B[Blue]\nstyle A fill:#ff5252\nstyle B fill:#2196f3", "mermaid");
    let (mut red, mut blue) = (false, false);
    for y in 0..30 {
        for x in 0..80 {
            if let Some(ratatui::style::Color::Rgb(r, g, b)) = buf[(x, y)].style().fg {
                if r == 255 && g == 82 && b == 82 { red = true; }
                if r == 33 && g == 150 && b == 243 { blue = true; }
            }
        }
    }
    assert!(red, "red node");
    assert!(blue, "blue node");
}

// ===== RL orientation =====
#[test]
fn flowchart_rl() {
    let (buf, _) = render("graph RL\nA[First] --> B[Second]", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 10));
    assert!(text.contains("First"));
    assert!(text.contains('◀'), "left arrow for RL");
}

// ===== BT orientation =====
#[test]
fn flowchart_bt() {
    let (buf, _) = render("graph BT\nA[Bottom] --> B[Top]", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 10));
    assert!(text.contains('▲'), "up arrow for BT");
}

// ===== Sequence =====
#[test]
fn sequence_basic() {
    let (buf, _) = render("sequenceDiagram\nparticipant Alice\nparticipant Bob\nAlice->>Bob: Hello\nBob-->>Alice: Hi", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 25));
    for lbl in ["Alice", "Bob", "Hello", "Hi"] {
        assert!(text.contains(lbl), "{} missing", lbl);
    }
    assert!(text.contains('┊'), "lifeline");
}

// ===== State =====
#[test]
fn state_basic() {
    let (buf, _) = render("stateDiagram-v2\n[*] --> Idle\nIdle --> Running : start\nRunning --> Done\nDone --> [*]", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 25));
    for lbl in ["Idle", "Running", "Done", "start"] {
        assert!(text.contains(lbl), "{} missing", lbl);
    }
}

// ===== Class =====
#[test]
fn class_basic() {
    let (buf, _) = render("classDiagram\nclass Animal {\n+String name\n+eat()\n}\nclass Dog {\n+bark()\n}\nAnimal <|-- Dog", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 25));
    for lbl in ["Animal", "Dog", "name", "eat", "bark"] {
        assert!(text.contains(lbl), "{} missing", lbl);
    }
}

// ===== Clipping =====
#[test]
fn clipped_to_area() {
    let area = Rect::new(10, 5, 20, 8);
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 30));
    let _ = render_block(&mut buf, area, "mermaid", "graph TD\nA[Start] --> B[End]");
    for y in 0..30 {
        for x in 0..80 {
            if x >= 10 && x < 30 && y >= 5 && y < 13 { continue; }
            assert_eq!(buf[(x, y)].symbol(), " ", "leaked at ({},{})", x, y);
        }
    }
}

// ===== CJK =====
#[test]
fn cjk_labels() {
    let (buf, _) = render("graph TD\n開始 --> 終了", "mermaid");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 10));
    assert!(text.contains("開始"));
    assert!(text.contains("終了"));
}

// ===== No crash on edge cases =====
#[test]
fn empty_mermaid() { let _ = render("graph TD\n", "mermaid"); }

#[test]
fn garbage_input() { let _ = render("blah blah not mermaid", "mermaid"); }

// ===== Complex: deep chain + branch + colors + labels =====
#[test]
fn complex_flowchart() {
    let src = "\
graph TD
    A[Request] --> B{Auth?}
    B -->|valid| C[Process]
    B -->|invalid| D[Reject]
    C --> E[(Database)]
    E --> F[Response]
    D --> F
    style A fill:#4caf50
    style D fill:#f44336
    style E fill:#ff9800";
    let (buf, h) = render(src, "mermaid");
    assert!(h > 5, "complex diagram should be tall enough");
    let text = buf_text(&buf, Rect::new(0, 0, 80, 30));
    for lbl in ["Request", "Auth?", "Process", "Reject", "Database", "Response", "valid", "invalid"] {
        assert!(text.contains(lbl), "complex: {} missing", lbl);
    }
}
