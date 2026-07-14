# terra

*A blazing-fast TUI Markdown editor with a live, side-by-side preview.*

`terra` is a terminal editor built with [ratatui](https://crates.io/crates/ratatui),
[pulldown-cmark](https://crates.io/crates/pulldown-cmark) and
[syntect](https://crates.io/crates/syntect). Edit Markdown on the left, see it
beautifully rendered — headings, lists, tables, code with syntax highlighting,
blockquotes, task lists, links, images, footnotes — on the right, instantly.

## Features

- Split-pane **live preview** (edit ⟷ rendered), with proportional scroll-sync
- Word-wrap editor with line-number gutter
- **Search** with live match highlighting (`/`, `n`, `N`), Esc clears
- **Outline jump** (`Ctrl+O`) — list of headings, `j/k` + `Enter`
- Go-to-line (`:42`), command mode (`:w :q :x`)
- Smart list continuation (Enter in a list continues it; empty item outdents)
- Auto-indent, duplicate line (`Ctrl+D`), delete line (`Ctrl+K`)
- **Undo/redo** (`Ctrl+Z` / `Ctrl+Shift+Z` / `Ctrl+R`), word-granularity coalescing
- Mouse support: scroll, and click-to-position the cursor
- 3 UI themes + 6 code-highlight themes, wrap toggle, help panel
- Headless `--dump` mode for scripting / testing without a TTY

## Install

```bash
cargo install --path .        # or: cargo build --release && cp target/release/terra ~/.local/bin
```

## Usage

```bash
terra notes.md                # edit + preview
terra notes.md --read         # start in preview focus
terra --dump notes.md         # render one frame to stdout (no TTY)
terra --dump --search bold --width 80 --height 24 notes.md
```

## Keybindings

| Key | Action |
|---|---|
| `Ctrl+S` | save · `Ctrl+Q`/`Ctrl+C` quit |
| `Tab` | switch editor ⟷ preview |
| `Ctrl+O` | outline jump · `/` search · `n`/`N` next/prev |
| `:` | command mode (`:w` `:q` `:x` `:42`) |
| `Ctrl+D`/`Ctrl+K` | duplicate / delete line |
| `Ctrl+Z`/`Ctrl+R` | undo / redo · `Ctrl+W` wrap · `Ctrl+T` theme |
| `Ctrl+Y` | toggle preview scroll-sync · `Ctrl+H`/`?` help |

## Status

22 unit tests covering the editing core (insert/delete/backspace, word motion,
multiline, smart lists, undo/redo). ~10 ms startup, ~2.9 MB binary.

License: MIT.
