# HNT - Hacker News Terminal

[![CI](https://github.com/thijsvos/hnt/actions/workflows/ci.yml/badge.svg)](https://github.com/thijsvos/hnt/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/thijsvos/hnt)](https://github.com/thijsvos/hnt/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A dark-themed terminal client for [Hacker News](https://news.ycombinator.com), built in Rust.

> **Status**: actively developed, pre-1.0. The 0.4.x line is stable for daily use; expect occasional rough edges around new features.

Browse stories, read threaded comments, and open links — all from your terminal. No more squinting at the orange-and-white website.

![hnt demo](assets/demo.gif)

## Features

- **Dark theme** — Catppuccin Mocha-inspired colors with HN orange accents
- **Split-pane layout** — Stories on the left, comments on the right
- **7 feeds** — Top, New, Best, Ask HN, Show HN, Jobs, Pinned (your starred stories with resume position)
- **Story type badges** — Visual labels for Ask HN, Show HN, and Jobs posts
- **Threaded comments** — Depth-colored bars for visual tracking, collapse/expand
- **Vim-style navigation** — `j`/`k`, `g`/`G`, `Ctrl+d`/`Ctrl+u`
- **Search** — Algolia-powered full-text search across stories
- **Reader mode** — Read article content directly in the terminal
- **Quickjump link hints** — In reader mode, press `f` and every hyperlink gets a 1- or 2-character home-row label. Type the label to open it (`f` browser, `F` HNT reader, `y` copy URL via OSC 52 — works through SSH)
- **Prior discussions** — Press `h` to see past HN submissions of the same URL with their scores and dates
- **Read-state tracking** — Visited stories render dimmed; stories with new comments since your last visit get a `+N` badge. Persisted to `$XDG_DATA_HOME/hnt/read.json`
- **What's New filter** — In the comments pane, press `n` to cycle through "all → new since last visit → recent 24h → all". Shows only comments newer than the threshold, with their parent comments preserved so the thread still reads in context. Turns the `+N` badge into something you can act on without scrolling through 500 comments to find the new ones.
- **Open in browser** — Press `o` to open the story URL
- **Progressive loading** — Root comments appear instantly, children load in the background
- **Lazy pagination** — Stories load automatically as you scroll

## Installation

### Download a binary

Grab the latest release for your platform from [Releases](https://github.com/thijsvos/hnt/releases).

```bash
# macOS (Apple Silicon)
curl -L https://github.com/thijsvos/hnt/releases/latest/download/hnt-aarch64-apple-darwin -o hnt
chmod +x hnt
./hnt

# macOS (Intel)
curl -L https://github.com/thijsvos/hnt/releases/latest/download/hnt-x86_64-apple-darwin -o hnt
chmod +x hnt
./hnt

# Linux (x86_64)
curl -L https://github.com/thijsvos/hnt/releases/latest/download/hnt-x86_64-unknown-linux-gnu -o hnt
chmod +x hnt
./hnt
```

> **Windows**: not currently in the release matrix. Build from source — `crossterm` (the terminal layer) supports Windows, so the only blocker is that the release workflow doesn't cross-build for it yet.

### Install via cargo

```bash
cargo install --git https://github.com/thijsvos/hnt
```

The crate isn't on crates.io yet, so the `--git` form is the idiomatic install path.

### Build from source

Requires [Rust](https://rustup.rs/) 1.88+.

```bash
git clone https://github.com/thijsvos/hnt.git
cd hnt
cargo build --release
./target/release/hnt
```

## Keybindings

| Key | Action |
|---|---|
| `j` / `k` or arrows | Navigate up / down |
| `Enter` | Select story / toggle collapse |
| `o` | Open URL in browser |
| `p` | Open reader mode |
| `f` / `F` / `y` | In reader: Quickjump label hints — open in browser / open in reader / copy to clipboard |
| `b` | Pin / unpin focused story (★) |
| `h` | Show prior HN submissions of this URL |
| `/` | Search stories |
| `Tab` | Switch pane focus |
| `1`-`7` | Switch feed (Top/New/Best/Ask/Show/Jobs/Pinned) |
| `r` | Refresh |
| `n` | Cycle "What's New" filter (comments pane) |
| `g` / `G` | Jump to top / bottom |
| `Ctrl+d` / `Ctrl+u` | Page down / up |
| `q` | Quit |
| `Esc` | Back / close |
| `?` | Help overlay |

## Configuration & state

`hnt` is configuration-free — there's no config file. It persists two pieces of state across runs:

| What | File |
|---|---|
| Visited stories + comment counts at last visit (drives the dim styling and `+N` "what's new" badges) | `read.json` |
| Pinned stories + per-story resume position | `pinned.json` |

The directory is platform-dependent (XDG on Linux, Application Support on macOS, AppData on Windows):

| OS | Path |
|---|---|
| Linux | `$XDG_DATA_HOME/hnt/` (defaults to `~/.local/share/hnt/`) |
| macOS | `~/Library/Application Support/hnt/` |
| Windows | `%APPDATA%\hnt\` |

Both files are written atomically (tmp + rename) with mode `0600` on Unix; the parent directory is created on first write with mode `0700`. Deleting either file resets the corresponding state — no other side effects.

## Changelog

Per-release notes live in [CHANGELOG.md](CHANGELOG.md). The [latest release](https://github.com/thijsvos/hnt/releases/latest) page mirrors the same content.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

Found a bug or have an idea? [Open an issue](https://github.com/thijsvos/hnt/issues).

## License

[MIT](LICENSE)
