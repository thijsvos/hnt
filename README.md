# HNT - Hacker News Terminal

[![CI](https://github.com/thijsvos/hnt/actions/workflows/ci.yml/badge.svg)](https://github.com/thijsvos/hnt/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/thijsvos/hnt)](https://github.com/thijsvos/hnt/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A dark-themed terminal client for [Hacker News](https://news.ycombinator.com), built in Rust.

Browse stories, read threaded comments, and open links — all from your terminal. No more squinting at the orange-and-white website.

![hnt demo](assets/demo.gif)

## Features

- **Dark theme** — Catppuccin Mocha-inspired colors with HN orange accents
- **Split-pane layout** — Stories on the left, comments on the right
- **6 feeds** — Top, New, Best, Ask HN, Show HN, Jobs
- **Story type badges** — Visual labels for Ask HN, Show HN, and Jobs posts
- **Threaded comments** — Depth-colored bars for visual tracking, collapse/expand
- **Vim-style navigation** — `j`/`k`, `g`/`G`, `Ctrl+d`/`Ctrl+u`
- **Search** — Algolia-powered full-text search across stories
- **Reader mode** — Read article content directly in the terminal
- **Prior discussions** — Press `h` to see past HN submissions of the same URL with their scores and dates
- **Read-state tracking** — Visited stories render dimmed; stories with new comments since your last visit get a `+N` badge. Persisted to `$XDG_DATA_HOME/hnt/read.json`
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
| `h` | Show prior HN submissions of this URL |
| `/` | Search stories |
| `Tab` | Switch pane focus |
| `1`-`6` | Switch feed (Top/New/Best/Ask/Show/Jobs) |
| `r` | Refresh |
| `g` / `G` | Jump to top / bottom |
| `Ctrl+d` / `Ctrl+u` | Page down / up |
| `q` | Quit |
| `Esc` | Back / close |
| `?` | Help overlay |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

Found a bug or have an idea? [Open an issue](https://github.com/thijsvos/hnt/issues).

## License

[MIT](LICENSE)
