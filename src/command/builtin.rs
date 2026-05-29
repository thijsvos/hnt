//! Seed registration of all built-in `:`-commands.
//!
//! Each `register_*` helper takes a `&mut CommandRegistry` and pushes one
//! [`Command`]. The full set is registered in dependency-free order by
//! [`register_builtins`] — called once from
//! [`crate::command::CommandRegistry::with_builtins`] during
//! [`crate::app::App::new`].
//!
//! The function-pointer signature constrains each closure to direct
//! [`crate::app::App`] mutation (no captured state). For commands whose
//! effect is already an existing [`crate::keys::Action`], the closure
//! just calls [`crate::app::App::dispatch`]; for richer effects (`:yank`,
//! `:open`, `:filter`) the closure inlines the logic or delegates to a
//! dedicated `App` helper method.

use crate::api::types::{FeedKind, StoryId};
use crate::clipboard;
use crate::command::{Arity, Command, CommandRegistry, CommandResult};
use crate::keys::Action;
use crate::sanitize::sanitize_terminal;
use crate::state::comment_state::CommentFilter;
use crate::state::hint_state::HintAction;

/// Registers every built-in command into `r`. Idempotent only if `r`
/// starts empty — re-running on a populated registry stacks duplicates.
pub fn register_builtins(r: &mut CommandRegistry) {
    register_quit(r);
    register_refresh(r);
    register_help(r);
    register_feed(r);
    register_open(r);
    register_search(r);
    register_reader(r);
    register_pin(r);
    register_filter(r);
    register_yank(r);
    register_goto(r);
    register_hint(r);
}

fn register_quit(r: &mut CommandRegistry) {
    r.register(Command {
        name: "quit",
        aliases: &["q"],
        description: "Quit hnt",
        arity: Arity::Exact(0),
        arg_completions: &[],
        run: |app, _args| {
            app.dispatch(Action::Quit);
            CommandResult::Ok
        },
    });
}

fn register_refresh(r: &mut CommandRegistry) {
    r.register(Command {
        name: "refresh",
        aliases: &["r"],
        description: "Reload the current feed (or rerun the current search)",
        arity: Arity::Exact(0),
        arg_completions: &[],
        run: |app, _args| {
            app.dispatch(Action::Refresh);
            CommandResult::Ok
        },
    });
}

fn register_help(r: &mut CommandRegistry) {
    r.register(Command {
        name: "help",
        aliases: &["h"],
        description: "List available commands",
        arity: Arity::Variadic,
        arg_completions: &[],
        run: |app, _args| {
            let names: Vec<&str> = app.command_registry.all().iter().map(|c| c.name).collect();
            CommandResult::OkInfo(format!("Commands: :{}", names.join("  :")))
        },
    });
}

fn register_feed(r: &mut CommandRegistry) {
    r.register(Command {
        name: "feed",
        aliases: &[],
        description: "Switch feed: top, new, best, ask, show, jobs, pinned",
        arity: Arity::Exact(1),
        arg_completions: &["top", "new", "best", "ask", "show", "jobs", "pinned"],
        run: |app, args| {
            let name = args[0].to_lowercase();
            let idx = match feed_index(&name) {
                Some(i) => i,
                None => {
                    return CommandResult::Err(format!(
                        "Unknown feed: {name}. Try: top, new, best, ask, show, jobs, pinned"
                    ))
                }
            };
            app.dispatch(Action::SwitchFeed(idx));
            CommandResult::Ok
        },
    });
}

fn register_open(r: &mut CommandRegistry) {
    r.register(Command {
        name: "open",
        aliases: &[],
        description: "Open a story by HN item ID (e.g. :open 12345)",
        arity: Arity::Exact(1),
        arg_completions: &[],
        run: |app, args| {
            let id: u64 = match args[0].parse() {
                Ok(n) => n,
                Err(_) => return CommandResult::Err(format!("Invalid story ID: {}", args[0])),
            };
            app.spawn_open_story_by_id(id);
            CommandResult::OkInfo(format!("Loading story {id}…"))
        },
    });
}

fn register_search(r: &mut CommandRegistry) {
    r.register(Command {
        name: "search",
        aliases: &[],
        description: "Algolia search across all of HN (e.g. :search rust async)",
        arity: Arity::AtLeast(1),
        arg_completions: &[],
        run: |app, args| {
            let query = args.join(" ");
            if query.trim().is_empty() {
                return CommandResult::Err("Empty search query".to_string());
            }
            app.enter_search_mode();
            if let Some(ss) = app.search_state.as_mut() {
                ss.input = query;
            }
            app.submit_search();
            CommandResult::Ok
        },
    });
}

fn register_reader(r: &mut CommandRegistry) {
    r.register(Command {
        name: "reader",
        aliases: &[],
        description: "Open the focused story in the inline article reader",
        arity: Arity::Exact(0),
        arg_completions: &[],
        run: |app, _args| {
            app.dispatch(Action::OpenReader);
            CommandResult::Ok
        },
    });
}

fn register_pin(r: &mut CommandRegistry) {
    r.register(Command {
        name: "pin",
        // `b` is the existing key binding, kept as an alias for muscle memory.
        // `:unpin` reaches the same toggle; the user just speaks the intent
        // they have, and the toggle figures it out.
        aliases: &["unpin", "b"],
        description: "Toggle the pinned state of the focused story",
        arity: Arity::Exact(0),
        arg_completions: &[],
        run: |app, _args| {
            app.dispatch(Action::TogglePin);
            CommandResult::Ok
        },
    });
}

fn register_filter(r: &mut CommandRegistry) {
    r.register(Command {
        name: "filter",
        aliases: &[],
        description: "Set comment filter: all, new, 24h, author <user>",
        arity: Arity::AtLeast(1),
        arg_completions: &["all", "new", "24h", "author"],
        run: |app, args| {
            let mode = args[0].to_lowercase();
            let Some(story) = app.comment_state.story.as_ref() else {
                return CommandResult::Err(
                    "No story loaded — open a story's comments first".to_string(),
                );
            };
            let story_id = StoryId(story.id);
            let new_filter = match mode.as_str() {
                "all" => CommentFilter::All,
                "new" => match app.read_store.last_seen_at(story_id) {
                    Some(t) => CommentFilter::NewSince(t),
                    None => {
                        return CommandResult::Err(
                            "This story hasn't been visited before — no `new` anchor".to_string(),
                        )
                    }
                },
                "24h" => {
                    // Match cycle_comment_filter's 60s skew tolerance so
                    // typed and cycled filters behave identically.
                    let cutoff = chrono::Utc::now().timestamp() - 86_400 - 60;
                    CommentFilter::Recent(cutoff)
                }
                "author" => {
                    if args.len() < 2 {
                        return CommandResult::Err(
                            ":filter author requires a username (e.g. :filter author dang)"
                                .to_string(),
                        );
                    }
                    // HN usernames are case-sensitive — preserve `args[1]`
                    // as typed. Trim so a trailing space doesn't silently
                    // break the match.
                    let username = args[1].trim();
                    if username.is_empty() {
                        // A quoted empty/whitespace arg (`:filter author ""`)
                        // satisfies the count guard but would install an
                        // Author("") that matches nothing and blanks the
                        // thread with no feedback — reject it explicitly.
                        return CommandResult::Err(
                            ":filter author requires a non-empty username".to_string(),
                        );
                    }
                    CommentFilter::Author(username.to_string())
                }
                other => {
                    return CommandResult::Err(format!(
                        "Unknown filter: {other}. Try: all, new, 24h, author <user>"
                    ))
                }
            };
            app.comment_state.filter = new_filter;
            let visible = app.comment_state.visible_len();
            if visible == 0 {
                app.comment_state.selected = 0;
            } else if app.comment_state.selected >= visible {
                app.comment_state.selected = visible - 1;
            }
            CommandResult::Ok
        },
    });
}

fn register_yank(r: &mut CommandRegistry) {
    r.register(Command {
        name: "yank",
        aliases: &["y"],
        description: "Copy to clipboard via OSC 52: url, title, or both",
        arity: Arity::Exact(1),
        arg_completions: &["url", "title", "both"],
        run: |app, args| {
            let field = args[0].to_lowercase();
            let Some(story) = app.focused_story() else {
                return CommandResult::Err("No focused story".to_string());
            };
            let title = story.title.clone().unwrap_or_default();
            let url = story
                .url
                .clone()
                .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={}", story.id));
            let text = match field.as_str() {
                "url" => url,
                "title" => title,
                "both" => format!("{title}\n{url}"),
                other => {
                    return CommandResult::Err(format!(
                        "Unknown field: {other}. Try: url, title, both"
                    ))
                }
            };
            match clipboard::copy(&text) {
                Ok(()) => {
                    // Sanitise the echoed text — story titles/URLs are
                    // server-controlled and we surface them in the status bar.
                    CommandResult::OkInfo(format!("Copied: {}", sanitize_terminal(&text)))
                }
                Err(e) => CommandResult::Err(format!("Clipboard write failed: {e}")),
            }
        },
    });
}

fn register_goto(r: &mut CommandRegistry) {
    r.register(Command {
        name: "goto",
        aliases: &["g"],
        description: "Jump in the focused pane: top, bottom",
        arity: Arity::Exact(1),
        arg_completions: &["top", "bottom"],
        run: |app, args| {
            let target = args[0].to_lowercase();
            let action = match target.as_str() {
                "top" => Action::JumpTop,
                "bottom" => Action::JumpBottom,
                other => {
                    return CommandResult::Err(format!(
                        "Unknown goto target: {other}. Try: top, bottom"
                    ))
                }
            };
            app.dispatch(action);
            CommandResult::Ok
        },
    });
}

fn register_hint(r: &mut CommandRegistry) {
    r.register(Command {
        name: "hint",
        aliases: &[],
        description: "Enter Quickjump hint mode: open, reader, yank",
        arity: Arity::Exact(1),
        arg_completions: &["open", "reader", "yank"],
        run: |app, args| {
            let mode = args[0].to_lowercase();
            let hint = match mode.as_str() {
                "open" => HintAction::Open,
                "reader" => HintAction::OpenInReader,
                "yank" => HintAction::CopyUrl,
                other => {
                    return CommandResult::Err(format!(
                        "Unknown hint action: {other}. Try: open, reader, yank"
                    ))
                }
            };
            app.dispatch(Action::EnterHintMode(hint));
            CommandResult::Ok
        },
    });
}

/// Maps a feed name (case-folded by caller) to its index in
/// [`FeedKind::ALL`]. Returns `None` for unknown names.
fn feed_index(name: &str) -> Option<usize> {
    FeedKind::ALL
        .iter()
        .position(|f| f.to_string().eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    fn run_cmd(app: &mut App, line: &str) {
        let registry = CommandRegistry::with_builtins();
        let (name, args) = crate::command::parser::parse(line).expect("parse");
        let cmd = registry.lookup(&name).expect("command registered");
        registry
            .check_arity(cmd, &args)
            .expect("arity ok in test setup");
        (cmd.run)(app, &args);
    }

    #[test]
    fn feed_index_recognises_every_feedkind() {
        for (i, f) in FeedKind::ALL.iter().enumerate() {
            let name = f.to_string().to_lowercase();
            assert_eq!(feed_index(&name), Some(i), "{name} should map to {i}");
        }
    }

    #[test]
    fn feed_index_rejects_unknown() {
        assert_eq!(feed_index("nope"), None);
    }

    #[test]
    fn feed_arg_completions_are_all_valid_feeds() {
        // Drift guard: every Tab-completion candidate for `:feed` must be a
        // real feed (resolve via feed_index), and the count must match
        // FeedKind::ALL so a new feed isn't silently left uncompletable.
        let registry = CommandRegistry::with_builtins();
        let feed = registry.lookup("feed").unwrap();
        assert_eq!(
            feed.arg_completions.len(),
            FeedKind::ALL.len(),
            "feed completion list out of sync with FeedKind::ALL"
        );
        for cand in feed.arg_completions {
            assert!(feed_index(cand).is_some(), "{cand:?} is not a valid feed");
        }
    }

    #[tokio::test]
    async fn refresh_command_dispatches_action_refresh() {
        // Refresh triggers reset_panes_and_reload which spawns an async
        // fetch — needs a running tokio runtime, same as the existing
        // `reset_panes_and_reload_*` tests.
        let mut app = App::new(80, 24);
        run_cmd(&mut app, "refresh");
        assert!(app.running);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn feed_command_switches_current_feed() {
        let mut app = App::new(80, 24);
        assert_eq!(app.current_feed, FeedKind::Top);
        run_cmd(&mut app, "feed best");
        assert_eq!(app.current_feed, FeedKind::Best);
    }

    #[tokio::test]
    async fn feed_command_is_case_insensitive() {
        let mut app = App::new(80, 24);
        run_cmd(&mut app, "feed BEST");
        assert_eq!(app.current_feed, FeedKind::Best);
    }

    #[test]
    fn feed_command_unknown_name_errors() {
        let mut app = App::new(80, 24);
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("feed").unwrap();
        let result = (cmd.run)(&mut app, &["xyz".to_string()]);
        assert!(matches!(result, CommandResult::Err(_)));
        assert_eq!(
            app.current_feed,
            FeedKind::Top,
            "feed must not change on err"
        );
    }

    #[test]
    fn open_command_rejects_non_numeric_id() {
        let mut app = App::new(80, 24);
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("open").unwrap();
        let result = (cmd.run)(&mut app, &["abc".to_string()]);
        match result {
            CommandResult::Err(msg) => assert!(msg.contains("Invalid")),
            _ => panic!("expected Err for non-numeric ID"),
        }
    }

    #[test]
    fn reader_command_dispatches_open_reader() {
        let mut app = App::new(80, 24);
        // No focused story → OpenReader is a no-op but does not crash.
        run_cmd(&mut app, "reader");
        assert!(app.running);
    }

    #[test]
    fn pin_command_aliases_resolve() {
        let registry = CommandRegistry::with_builtins();
        assert!(registry.lookup("pin").is_some());
        assert!(registry.lookup("unpin").is_some());
        assert!(registry.lookup("b").is_some());
    }

    #[test]
    fn filter_command_errors_when_no_story_loaded() {
        let mut app = App::new(80, 24);
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("filter").unwrap();
        let result = (cmd.run)(&mut app, &["all".to_string()]);
        assert!(matches!(result, CommandResult::Err(_)));
    }

    #[test]
    fn filter_command_author_sets_author_filter() {
        let mut app = App::new(80, 24);
        use crate::api::types::Item;
        use std::sync::Arc;
        app.comment_state.story = Some(Arc::new(Item {
            id: 1,
            title: Some("t".into()),
            url: None,
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: None,
            dead: None,
            deleted: None,
        }));
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("filter").unwrap();
        let result = (cmd.run)(&mut app, &["author".to_string(), "dang".to_string()]);
        assert!(matches!(result, CommandResult::Ok));
        assert_eq!(
            app.comment_state.filter,
            CommentFilter::Author("dang".into())
        );
    }

    #[test]
    fn filter_command_author_without_username_errors() {
        let mut app = App::new(80, 24);
        use crate::api::types::Item;
        use std::sync::Arc;
        app.comment_state.story = Some(Arc::new(Item {
            id: 1,
            title: Some("t".into()),
            url: None,
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: None,
            dead: None,
            deleted: None,
        }));
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("filter").unwrap();
        let result = (cmd.run)(&mut app, &["author".to_string()]);
        match result {
            CommandResult::Err(msg) => assert!(msg.contains("username")),
            _ => panic!("expected Err"),
        }
    }

    #[test]
    fn filter_command_author_rejects_empty_username() {
        // A quoted empty / whitespace-only arg (`:filter author "  "`)
        // satisfies the count guard but must still be rejected rather than
        // installing an Author("") that silently blanks the thread.
        let mut app = App::new(80, 24);
        use crate::api::types::Item;
        use std::sync::Arc;
        app.comment_state.story = Some(Arc::new(Item {
            id: 1,
            title: Some("t".into()),
            url: None,
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: None,
            dead: None,
            deleted: None,
        }));
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("filter").unwrap();
        let result = (cmd.run)(&mut app, &["author".to_string(), "   ".to_string()]);
        match result {
            CommandResult::Err(msg) => assert!(msg.contains("non-empty"), "got {msg}"),
            _ => panic!("expected Err for empty username"),
        }
        assert_eq!(
            app.comment_state.filter,
            CommentFilter::All,
            "filter must be unchanged on rejection"
        );
    }

    #[test]
    fn filter_command_unknown_mode_errors() {
        let mut app = App::new(80, 24);
        // Force a story loaded so we reach the mode-parse branch.
        use crate::api::types::Item;
        use std::sync::Arc;
        app.comment_state.story = Some(Arc::new(Item {
            id: 1,
            title: Some("t".into()),
            url: None,
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: None,
            dead: None,
            deleted: None,
        }));
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("filter").unwrap();
        let result = (cmd.run)(&mut app, &["xyz".to_string()]);
        assert!(matches!(result, CommandResult::Err(_)));
    }

    #[test]
    fn yank_command_errors_when_no_focused_story() {
        let mut app = App::new(80, 24);
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("yank").unwrap();
        let result = (cmd.run)(&mut app, &["url".to_string()]);
        assert!(matches!(result, CommandResult::Err(_)));
    }

    #[test]
    fn yank_command_rejects_unknown_field() {
        let mut app = App::new(80, 24);
        use crate::api::types::Item;
        use std::sync::Arc;
        app.story_state.stories.push(Arc::new(Item {
            id: 1,
            title: Some("t".into()),
            url: Some("https://example.com".into()),
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: None,
            dead: None,
            deleted: None,
        }));
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("yank").unwrap();
        let result = (cmd.run)(&mut app, &["sideways".to_string()]);
        assert!(matches!(result, CommandResult::Err(_)));
    }

    #[test]
    fn goto_command_top_bottom_recognised() {
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("goto").unwrap();
        let mut app = App::new(80, 24);
        let r = (cmd.run)(&mut app, &["top".to_string()]);
        assert!(matches!(r, CommandResult::Ok));
        let r = (cmd.run)(&mut app, &["bottom".to_string()]);
        assert!(matches!(r, CommandResult::Ok));
        let r = (cmd.run)(&mut app, &["sideways".to_string()]);
        assert!(matches!(r, CommandResult::Err(_)));
    }

    #[test]
    fn hint_command_recognises_all_three_actions() {
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("hint").unwrap();
        let mut app = App::new(80, 24);
        for action in ["open", "reader", "yank"] {
            let r = (cmd.run)(&mut app, &[action.to_string()]);
            assert!(matches!(r, CommandResult::Ok), "{action} should be Ok");
        }
        let r = (cmd.run)(&mut app, &["unknown".to_string()]);
        assert!(matches!(r, CommandResult::Err(_)));
    }

    #[test]
    fn help_command_returns_command_list() {
        let mut app = App::new(80, 24);
        let registry = CommandRegistry::with_builtins();
        let cmd = registry.lookup("help").unwrap();
        let result = (cmd.run)(&mut app, &[]);
        match result {
            CommandResult::OkInfo(msg) => {
                assert!(msg.contains(":quit"));
                assert!(msg.contains(":feed"));
                assert!(msg.contains(":filter"));
            }
            _ => panic!("help must return OkInfo with command list"),
        }
    }

    #[test]
    fn all_seeded_commands_registered() {
        let registry = CommandRegistry::with_builtins();
        for name in [
            "quit", "refresh", "help", "feed", "open", "search", "reader", "pin", "filter", "yank",
            "goto", "hint",
        ] {
            assert!(registry.lookup(name).is_some(), "{name} missing");
        }
    }
}
