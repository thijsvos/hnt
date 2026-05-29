//! Vim-style `:`-command registry.
//!
//! Each [`Command`] is a name + aliases + arity contract + closure that
//! mutates [`crate::app::App`] directly. The [`CommandRegistry`] stores
//! the built-in set seeded by [`builtin::register_builtins`] and is
//! consulted by [`crate::app::App::submit_command`] and the
//! command-palette overlay.
//!
//! The function-pointer signature on [`Command::run`] deliberately avoids
//! capturing any closure state — every command's effect must be expressible
//! as direct calls on [`crate::app::App`]. This keeps the registry
//! `'static` and trivially `Send + Sync` without `Box<dyn Fn>` per
//! command.

use crate::app::App;

pub mod builtin;
pub mod parser;

/// Signature every `:command` effect implements — a bare `fn` pointer (no
/// captured state) so the registry stays `'static + Send + Sync`. Named so
/// call sites and [`CommandRegistry::resolve`] can pass it around without
/// repeating the (clippy-flagged) complex type.
pub type CommandFn = fn(&mut App, &[String]) -> CommandResult;

/// Per-command argument-count contract — enforced by
/// [`CommandRegistry::check_arity`] before [`Command::run`] is invoked so
/// each command's closure can index args without bounds checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    /// Exactly this many positional arguments required.
    Exact(usize),
    /// At least this many arguments; trailing args are the command's
    /// responsibility to join (e.g. `:search` joins them back into one query).
    AtLeast(usize),
    /// Any number of arguments, including zero.
    Variadic,
}

/// Outcome of running a command — drives status-bar feedback in
/// [`crate::app::App::submit_command`].
#[non_exhaustive]
pub enum CommandResult {
    /// Command succeeded with no message — keep the status bar as-is.
    Ok,
    /// Command succeeded with a transient info toast (rendered via
    /// [`crate::app::App::set_info`]).
    OkInfo(String),
    /// Command failed — the message is surfaced as a red status-bar error
    /// via [`crate::app::App::error`].
    Err(String),
}

/// A registered `:command` callable from the prompt or palette.
///
/// Stored by value in [`CommandRegistry::commands`]; all fields are
/// `'static` so the registry as a whole is `'static + Send + Sync`.
pub struct Command {
    /// Canonical name typed after `:` (no leading colon).
    pub name: &'static str,
    /// Alternate names that resolve to the same command via
    /// [`CommandRegistry::lookup`].
    pub aliases: &'static [&'static str],
    /// One-line description shown in `?`-help and the command palette.
    pub description: &'static str,
    /// Arity contract enforced before [`Self::run`] is invoked.
    pub arity: Arity,
    /// Static completion candidates for the **first** positional argument
    /// (e.g. `feed` → `top, new, …`). Empty means no arg completion —
    /// either the command takes no args or the arg is freeform (an ID, a
    /// search query, a username). Consumed by
    /// [`crate::app::App::complete_command_at_cursor`].
    pub arg_completions: &'static [&'static str],
    /// Effect closure — receives the [`App`] and parsed args by reference.
    pub run: CommandFn,
}

/// In-process registry of all `:`-commands. Built once in
/// [`crate::app::App::new`] via [`Self::with_builtins`].
#[derive(Default)]
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    /// Builds a registry seeded with every built-in command via
    /// [`builtin::register_builtins`].
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        builtin::register_builtins(&mut r);
        r
    }

    /// Appends a command. Last-write-wins on name conflicts is
    /// intentional — caller is responsible for not registering duplicates.
    pub fn register(&mut self, cmd: Command) {
        self.commands.push(cmd);
    }

    /// Resolves a typed name (or alias) to a registered command. `None`
    /// if no match.
    pub fn lookup(&self, name: &str) -> Option<&Command> {
        self.commands
            .iter()
            .find(|c| c.name == name || c.aliases.contains(&name))
    }

    /// All registered commands, in registration order.
    pub fn all(&self) -> &[Command] {
        &self.commands
    }

    /// Validates `args.len()` against the registered [`Arity`].
    /// Returns `Ok(())` if the count is acceptable, `Err(msg)` otherwise.
    pub fn check_arity(&self, cmd: &Command, args: &[String]) -> Result<(), String> {
        match cmd.arity {
            Arity::Exact(n) if args.len() != n => Err(format!(
                ":{} expects {n} arg(s), got {}",
                cmd.name,
                args.len()
            )),
            Arity::AtLeast(n) if args.len() < n => Err(format!(
                ":{} expects at least {n} arg(s), got {}",
                cmd.name,
                args.len()
            )),
            _ => Ok(()),
        }
    }

    /// Resolves a typed name (or alias) to its [`CommandFn`], validating
    /// arity in one step. Returns the fn pointer on success, or a
    /// user-facing error message (unknown command / arity mismatch).
    ///
    /// Lets callers run the command with `&mut App` without holding a
    /// `&Command` borrow across the call — folding the lookup + arity
    /// check + borrow dance that otherwise repeats at every entry point.
    pub fn resolve(&self, name: &str, args: &[String]) -> Result<CommandFn, String> {
        let cmd = self
            .lookup(name)
            .ok_or_else(|| format!("Unknown command: {name}"))?;
        self.check_arity(cmd, args)?;
        Ok(cmd.run)
    }

    /// Subsequence-fuzzy match against `query`. Returns `(index, score)`
    /// pairs into [`Self::all`], sorted by descending score: longer matches
    /// and earlier matches score higher, with a prefix bonus.
    ///
    /// Returns indices rather than `&Command` so callers (the palette) don't
    /// have to recover the index via a pointer-identity scan. Empty `query`
    /// returns every command with a score of zero (stable registration
    /// order) — the right behaviour for "opened palette with no input yet."
    pub fn fuzzy(&self, query: &str) -> Vec<(usize, i64)> {
        if query.is_empty() {
            return (0..self.commands.len()).map(|i| (i, 0)).collect();
        }
        let q = query.to_lowercase();
        let mut out: Vec<(usize, i64)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| score_subseq(&q, c.name).map(|s| (i, s)))
            .collect();
        out.sort_by_key(|b| std::cmp::Reverse(b.1));
        out
    }
}

/// Returns `Some(score)` if every char of `q` appears in `target` in
/// order (subsequence match), else `None`.
///
/// Scoring rewards consecutive matches and prefix matches so `re` ranks
/// `refresh` above `reader` above any subsequence-only match like
/// `[unrelated]`. `q` must already be lowercased by the caller; `target`
/// is matched case-insensitively via [`char::eq_ignore_ascii_case`], so
/// command names need no per-call lowercasing allocation.
fn score_subseq(q: &str, target: &str) -> Option<i64> {
    let mut q_chars = q.chars();
    let mut current = q_chars.next()?;
    let mut score: i64 = 0;
    let mut consec: i64 = 0;
    let mut matched_at_start = false;
    for (i, c) in target.chars().enumerate() {
        if c.eq_ignore_ascii_case(&current) {
            if i == 0 {
                matched_at_start = true;
            }
            score += 10 + consec * 5;
            consec += 1;
            match q_chars.next() {
                Some(next) => current = next,
                None => {
                    if matched_at_start {
                        score += 50;
                    }
                    return Some(score);
                }
            }
        } else {
            consec = 0;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(name: &'static str, aliases: &'static [&'static str], arity: Arity) -> Command {
        Command {
            name,
            aliases,
            description: "",
            arity,
            arg_completions: &[],
            run: |_app, _args| CommandResult::Ok,
        }
    }

    #[test]
    fn lookup_resolves_name_and_aliases() {
        let mut r = CommandRegistry::default();
        r.register(dummy("quit", &["q"], Arity::Exact(0)));
        assert_eq!(r.lookup("quit").map(|c| c.name), Some("quit"));
        assert_eq!(r.lookup("q").map(|c| c.name), Some("quit"));
        assert!(r.lookup("nope").is_none());
    }

    #[test]
    fn fuzzy_finds_subsequence_matches() {
        let mut r = CommandRegistry::default();
        r.register(dummy("refresh", &[], Arity::Exact(0)));
        r.register(dummy("reader", &[], Arity::Exact(0)));
        r.register(dummy("filter", &[], Arity::AtLeast(1)));
        let names: Vec<_> = r
            .fuzzy("re")
            .into_iter()
            .map(|(i, _)| r.all()[i].name)
            .collect();
        assert!(names.contains(&"refresh"), "re ⊏ refresh");
        assert!(names.contains(&"reader"), "re ⊏ reader");
        // 'r' is at position 5 of "filter" with no 'e' after it → not a subsequence.
        assert!(!names.contains(&"filter"), "re is NOT a subseq of filter");
    }

    #[test]
    fn fuzzy_prefix_match_outranks_late_match() {
        let mut r = CommandRegistry::default();
        r.register(dummy("refresh", &[], Arity::Exact(0)));
        r.register(dummy("preferred", &[], Arity::Exact(0))); // 're' appears at index 2
        let ranked: Vec<_> = r
            .fuzzy("re")
            .into_iter()
            .map(|(i, _)| r.all()[i].name)
            .collect();
        assert_eq!(ranked.first().copied(), Some("refresh"));
    }

    #[test]
    fn fuzzy_empty_query_returns_all() {
        let mut r = CommandRegistry::default();
        r.register(dummy("a", &[], Arity::Exact(0)));
        r.register(dummy("b", &[], Arity::Exact(0)));
        assert_eq!(r.fuzzy("").len(), 2);
    }

    #[test]
    fn fuzzy_is_case_insensitive() {
        let mut r = CommandRegistry::default();
        r.register(dummy("refresh", &[], Arity::Exact(0)));
        assert_eq!(r.fuzzy("RE").len(), 1);
        assert_eq!(r.fuzzy("Re").len(), 1);
    }

    #[test]
    fn check_arity_exact_rejects_wrong_count() {
        let r = CommandRegistry::default();
        let cmd = dummy("x", &[], Arity::Exact(1));
        assert!(r.check_arity(&cmd, &[]).is_err());
        assert!(r
            .check_arity(&cmd, &["a".to_string(), "b".to_string()])
            .is_err());
        assert!(r.check_arity(&cmd, &["a".to_string()]).is_ok());
    }

    #[test]
    fn check_arity_at_least_accepts_more() {
        let r = CommandRegistry::default();
        let cmd = dummy("x", &[], Arity::AtLeast(1));
        assert!(r.check_arity(&cmd, &[]).is_err());
        assert!(r.check_arity(&cmd, &["a".to_string()]).is_ok());
        assert!(r
            .check_arity(&cmd, &["a".to_string(), "b".to_string()])
            .is_ok());
    }

    #[test]
    fn check_arity_variadic_accepts_any_count() {
        let r = CommandRegistry::default();
        let cmd = dummy("x", &[], Arity::Variadic);
        assert!(r.check_arity(&cmd, &[]).is_ok());
        assert!(r
            .check_arity(&cmd, &["a".to_string(), "b".to_string()])
            .is_ok());
    }

    #[test]
    fn builtins_register_quit() {
        let r = CommandRegistry::with_builtins();
        assert!(r.lookup("quit").is_some(), "quit must be registered");
        assert!(r.lookup("q").is_some(), "q alias must resolve");
    }

    #[test]
    fn resolve_returns_fn_for_name_and_alias() {
        let r = CommandRegistry::with_builtins();
        assert!(r.resolve("quit", &[]).is_ok());
        assert!(r.resolve("q", &[]).is_ok(), "alias must resolve");
    }

    #[test]
    fn resolve_unknown_command_errors() {
        let r = CommandRegistry::with_builtins();
        let err = r.resolve("nope", &[]).unwrap_err();
        assert!(err.contains("Unknown command"), "got {err}");
    }

    #[test]
    fn resolve_wrong_arity_errors() {
        let r = CommandRegistry::with_builtins();
        let err = r.resolve("quit", &["extra".to_string()]).unwrap_err();
        assert!(err.contains("expects"), "got {err}");
    }
}
