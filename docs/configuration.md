# Configuration & State

`hnt` is configuration-free — there's no config file. It persists two pieces
of state across runs:

| File | What it stores | Created by |
|---|---|---|
| `read.json` | Stories you've opened, plus the comment count at the time of each visit. Drives the dim styling on visited rows and the `+N` "what's new" badge. | First time you press `Enter` on a story. |
| `pinned.json` | Stories you've pinned (`b`) plus their resume position (selected-comment index, collapsed subtree IDs). Backs the `Pinned` virtual feed. | First time you pin a story. |

## File locations

Per-platform data directory:

| OS | Path |
|---|---|
| Linux | `$XDG_DATA_HOME/hnt/` (defaults to `~/.local/share/hnt/`) |
| macOS | `~/Library/Application Support/hnt/` |
| Windows | `%APPDATA%\hnt\` |

## On-disk shape

Both files are JSON, schema-versioned, and written atomically:

1. The file is serialised to `<file>.tmp` first.
2. `<file>.tmp` is then atomically renamed over `<file>`.

On Unix, the directory is created with mode `0700` (owner-only) and files
with mode `0600`. A corrupt file (e.g. truncated by a crashed write) is
silently re-initialised to an empty store on the next load rather than
crashing the app.

Each store has a soft cap (currently 200 entries for `read.json`, 100 for
`pinned.json`); the oldest entries are evicted when the cap is reached.

## Resetting state

Delete either file to reset the corresponding state. No other side effects;
the app will recreate them on next use.

## Why no config file?

The keymap, feed list, color palette, and tunables are all currently
compile-time constants. If you want different defaults, fork and rebuild.
A proper config layer is a possible 1.0 milestone; see issue tracker.
