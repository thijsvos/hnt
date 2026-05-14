# Docs

Supplementary documentation for `hnt`. The main README at the repository
root is the canonical starting point — these files exist to keep deeper
explanations out of it.

- [Configuration & state](configuration.md) — where `read.json` and
  `pinned.json` live per platform; how state is persisted and reset.
- [Internals](internals.md) — load-bearing-but-non-obvious mechanisms:
  the async architecture, generation-counter result gating, the SSRF
  guard, terminal-escape sanitisation, Quickjump labels, comment-tree
  memoisation.
