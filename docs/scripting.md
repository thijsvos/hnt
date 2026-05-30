# Scripting (headless mode)

`hnt` runs as a non-interactive CLI whenever it's given a subcommand: it
fetches, prints to stdout, and exits without ever entering raw mode or the
alternate screen. With **no** arguments it launches the interactive TUI as
before. This makes Hacker News scriptable — pipe it into `jq`, `fzf`, `cron`,
`mail`, a pager, or your editor.

There is no new network surface: headless mode reuses the same Firebase
Hacker News API and Algolia search the TUI uses, with the same 15-second
request timeout and SSRF guard for `article`. There is no config file and no
API key — output is local-only.

## Commands

| Command | Aliases | Description |
|---|---|---|
| `hnt <feed>` / `hnt feed <name>` | — | List a feed (`top` `new` `best` `ask` `show` `jobs` `pinned`) |
| `hnt thread <id>` | `comments` | Print a story header and its comment tree |
| `hnt open <id>` | `item` | Print a single item (story or comment) |
| `hnt search <query…>` | — | Algolia full-text search across all of HN |
| `hnt article <id\|url>` | `read` | Extract and print article text |
| `hnt --help` | `-h`, `help` | Usage |
| `hnt --version` | `-V`, `version` | Version |

`pinned` reads ids from your local pin store (`pinned.json`) — the same pins
you star with `b` in the TUI — then fetches them, so it works fully offline of
any feed endpoint.

## Options

| Option | Applies to | Meaning |
|---|---|---|
| `--json` | feeds, `thread`, `open`, `search` | Emit JSON (see contract below) instead of text |
| `--digest` | feeds | One compact line per story |
| `--limit N` | feeds, `search`, `thread` | Max items; for `thread`, caps top-level comments. Default 30 for listings |
| `--max-depth N` | `thread` | Max comment nesting. `0` = root comments only. Default 12 |

`--limit` and `--max-depth` also accept `--flag=N`. `--json` and `--digest`
are mutually exclusive.

## JSON contract

The `--json` shapes are deliberately decoupled from the internal wire types,
so scripts that depend on them won't break when internals change. Unset
optional fields are **omitted** rather than emitted as `null`.

**Story** (feed listings, `open`, and `search` results — an array):

| Field | Type | Notes |
|---|---|---|
| `id` | number | HN item id |
| `title` | string | Full title (badge prefix preserved) |
| `by` | string? | Submitter |
| `score` | number? | Points |
| `comments` | number? | Total comment count (HN `descendants`) |
| `time` | number? | Unix seconds |
| `url` | string? | External link; omitted for text posts |
| `domain` | string? | Host of `url`, `www.` stripped |
| `hn_url` | string | Discussion permalink (always present) |
| `type` | string? | `story` / `comment` / `job` / `poll` / … |

**Thread** (`hnt thread <id> --json`): `{ "story": <Story>, "comments": [<Comment>] }`,
where each **Comment** is `{ id, depth, by?, time?, text }` in pre-order
(`depth` 0 = a top-level reply). `text` is the HTML body rendered to plain
text.

## Exit status

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | A requested item/story was not found, or a runtime (network) error |
| `2` | Usage error (unknown command, bad flag, malformed value) |

Errors print to stderr. Running `hnt` with no subcommand while stdout is not a
terminal (a pipe or file) is treated as a usage error rather than dumping TUI
escape sequences into the stream.

## Safety

All text output is plain — no ANSI styling — and every Hacker-News-supplied
string is scrubbed of terminal control sequences before printing (the same
sanitisation the TUI applies on render). A title or comment can't smuggle an
escape sequence through a pipe into a later `cat`, pager, or editor.

## Examples

```bash
# Front-page titles, newline-separated
hnt top --limit 10 --json | jq -r '.[].title'

# Stories above 200 points right now
hnt best --limit 50 --json | jq -r '.[] | select(.score > 200) | "\(.score)\t\(.title)"'

# Read an article without leaving the terminal
hnt article 38911 | less

# Dump a whole thread to a file (full depth)
hnt thread 38911 --max-depth 99 > thread.txt

# Daily digest by email (cron)
hnt top --digest | mail -s "HN $(date +%F)" you@example.com

# fzf story picker → comments
id=$(hnt top --limit 30 --json | jq -r '.[] | "\(.id)\t\(.title)"' | fzf | cut -f1)
[ -n "$id" ] && hnt thread "$id" | less

# Who's hiring this month, as JSON
hnt jobs --limit 100 --json > jobs.json
```
