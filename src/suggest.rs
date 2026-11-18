//! "Did you mean ...?" for mistyped CLI commands.

/// Words `crew` routes itself in `run` (internal `__` commands and flag spellings excluded).
#[cfg(test)]
const TOP_LEVEL: &[&str] = &[
    "machine",
    "federation",
    "server",
    "doctor",
    "orchestrator",
    "workflow",
    "task",
    "integration",
    "plugin",
    "new",
    "new-session",
    "attach",
    "a",
    "attach-session",
    "attach-workspace",
    "ls",
    "list-sessions",
    "kill-session",
    "kill-server",
    "keys",
    "list-keys",
    "version",
    "help",
];

/// Every command word a user can type after `crew`: the top-level words above, the JSON API
/// groups (`api::cli::API_COMMANDS`) and the server command names and aliases that
/// `commands::parse` accepts. Canonical names come before their aliases so ties prefer them.
pub const COMMANDS: &[&str] = &[
    // top level
    "machine",
    "federation",
    "server",
    "doctor",
    "orchestrator",
    "workflow",
    "task",
    "integration",
    "plugin",
    "new",
    "new-session",
    "attach",
    "attach-session",
    "a",
    "attach-workspace",
    "list-sessions",
    "ls",
    "keys",
    "version",
    "help",
    // JSON API groups
    "pane",
    "agent",
    "api",
    "session",
    "workspace",
    "tab",
    "worktree",
    "events",
    "peer",
    "msg",
    // server commands
    "swap-pane",
    "move-window",
    "split-window",
    "splitw",
    "new-window",
    "neww",
    "rename-window",
    "renamew",
    "rename-session",
    "rename",
    "kill-pane",
    "killp",
    "kill-window",
    "killw",
    "kill-session",
    "kill-server",
    "select-pane",
    "selectp",
    "next-pane",
    "last-pane",
    "toggle-auto-focus",
    "select-layout",
    "next-layout",
    "resize-pane",
    "resizep",
    "next-window",
    "next",
    "previous-window",
    "prev",
    "select-window",
    "selectw",
    "copy-mode",
    "paste-buffer",
    "pasteb",
    "resume-pane",
    "detach-client",
    "detach",
    "choose-tree",
    "choose-session",
    "choose-window",
    "toggle-sidebar",
    "sidebar",
    "toggle-workspace-tree",
    "source-file",
    "source",
    "set-option",
    "set",
    "set-window-option",
    "setw",
    "bind-key",
    "bind",
    "unbind-key",
    "unbind",
    "command-prompt",
    "switch-client",
    "switchc",
    "send-prefix",
    "display-message",
    "display",
    "list-keys",
    "lsk",
    "debug-frame",
    "next-attention",
    "confirm-before",
    "confirm",
    "capture-pane",
    "capturep",
    "send-keys",
    "send",
];

/// Levenshtein distance over chars.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if ca == *cb {
                diagonal
            } else {
                1 + diagonal.min(above).min(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}

/// The candidate closest to `input`: the smallest edit distance up to 2 (1 for inputs of at
/// most 3 chars, and never a distance that rewrites the whole candidate), else the only
/// candidate that `input` (3+ chars) is a prefix of. Ties keep the earliest candidate.
pub fn nearest<'a>(input: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let input = input.to_ascii_lowercase();
    let len = input.chars().count();
    if len == 0 {
        return None;
    }
    let limit = if len <= 3 { 1 } else { 2 };
    let mut best: Option<(usize, &'a str)> = None;
    for &candidate in candidates {
        let d = distance(&input, candidate);
        if d <= limit && d < candidate.chars().count() && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, candidate));
        }
    }
    if let Some((_, candidate)) = best {
        return Some(candidate);
    }
    if len < 3 {
        return None;
    }
    let mut prefixed = candidates.iter().filter(|c| c.starts_with(&input));
    match (prefixed.next(), prefixed.next()) {
        (Some(only), None) => Some(*only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_close_typos_only() {
        for (typo, want) in [
            ("spilt-window", "split-window"),
            ("split-windw", "split-window"),
            ("SPLIT-WINDOW", "split-window"),
            ("doctr", "doctor"),
            ("lss", "ls"),
            ("atach", "attach"),
            ("kill-sever", "kill-server"),
            ("capture-pan", "capture-pane"),
            ("toggle-work", "toggle-workspace-tree"),
        ] {
            assert_eq!(nearest(typo, COMMANDS), Some(want), "{typo}");
        }
        for word in ["frobnicate", "xyzzy", "b", "qq", "", "zzzzzzzzzzzz"] {
            assert_eq!(nearest(word, COMMANDS), None, "{word}");
        }
        // Ambiguous prefixes suggest nothing; ties keep the earliest candidate.
        assert_eq!(nearest("toggle", COMMANDS), None);
        assert_eq!(nearest("abcd", &["abxx", "abyy"]), Some("abxx"));
    }

    #[test]
    fn distance_counts_edits() {
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("spilt", "split"), 2);
        assert_eq!(distance("same", "same"), 0);
    }

    #[test]
    fn every_candidate_is_routable() {
        let mut seen = std::collections::HashSet::new();
        for word in COMMANDS {
            assert!(seen.insert(word), "duplicate candidate {word}");
            let routed = TOP_LEVEL.contains(word)
                || crate::api::cli::API_COMMANDS.contains(word)
                || !crate::commands::parse(word).is_err_and(|e| e.starts_with("unknown command"));
            assert!(routed, "{word} is not a crew command");
        }
        for word in TOP_LEVEL.iter().chain(crate::api::cli::API_COMMANDS) {
            assert!(COMMANDS.contains(word), "{word} is missing from COMMANDS");
        }
    }
}
