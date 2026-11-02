pub type RunFn = fn(&[String]) -> i32;

pub struct Utility {
    pub name: &'static str,
    pub run: RunFn,
    pub summary: &'static str,
    pub origin: Origin,
    pub windows_conflict: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Existing,
    Ms,
    Extra,
}

pub static UTILITIES: &[Utility] = &[
    Utility {
        name: "[",
        run: crate::commands::test::run_bracket,
        summary: "Evaluate a bracketed expression",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "basename",
        run: crate::commands::pathname::run_basename,
        summary: "Remove directory prefix",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "cat",
        run: crate::text_commands::cat,
        summary: "Concatenate files",
        origin: Origin::Existing,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "cd",
        run: crate::cd_command::run,
        summary: "Resolve a directory",
        origin: Origin::Existing,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "comm",
        run: crate::commands::comm::run,
        summary: "Compare sorted files",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "curl",
        run: crate::curl_command::run,
        summary: "Transfer data from URLs",
        origin: Origin::Existing,
        windows_conflict: Some("PowerShell 5.1"),
    },
    Utility {
        name: "cut",
        run: crate::commands::cut::run,
        summary: "Select fields or bytes",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "df",
        run: crate::df_command::run,
        summary: "Report filesystem space",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "diff",
        run: crate::diff::run,
        summary: "Compare files",
        origin: Origin::Existing,
        windows_conflict: Some("PowerShell 5.1"),
    },
    Utility {
        name: "dirname",
        run: crate::commands::pathname::run_dirname,
        summary: "Remove final path component",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "echo",
        run: crate::commands::echo::run,
        summary: "Display a line of text",
        origin: Origin::Ms,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "egrep",
        run: crate::commands::egrep::run,
        summary: "Search extended regular expressions",
        origin: Origin::Extra,
        windows_conflict: None,
    },
    Utility {
        name: "env",
        run: crate::commands::env::run,
        summary: "Run a command in a modified environment",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "expand",
        run: crate::commands::expand::run_expand,
        summary: "Convert expand to spaces",
        origin: Origin::Ms,
        windows_conflict: Some("Windows System32"),
    },
    Utility {
        name: "false",
        run: crate::commands::truefalse::run_false,
        summary: "Exit unsuccessfully",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "fgrep",
        run: crate::commands::fgrep::run,
        summary: "Search fixed strings",
        origin: Origin::Extra,
        windows_conflict: None,
    },
    Utility {
        name: "find",
        run: crate::find_command::find,
        summary: "Find files",
        origin: Origin::Existing,
        windows_conflict: Some("Windows"),
    },
    Utility {
        name: "fold",
        run: crate::commands::fold::run,
        summary: "Wrap input lines",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "grep",
        run: crate::grep_command::run,
        summary: "Search text",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "head",
        run: crate::text_commands::head,
        summary: "Print first lines",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "less",
        run: crate::less_command::run,
        summary: "Display files",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "list",
        run: crate::files::list,
        summary: "List files recursively",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "ls",
        run: crate::fs_commands::ls,
        summary: "List directory contents",
        origin: Origin::Existing,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "nl",
        run: crate::commands::nl::run,
        summary: "Number input lines",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "paste",
        run: crate::commands::paste::run,
        summary: "Merge lines",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "printenv",
        run: crate::commands::printenv::run,
        summary: "Print environment values",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "printf",
        run: crate::commands::printf::run,
        summary: "Format arguments",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "ps",
        run: crate::ps_command::run,
        summary: "List processes",
        origin: Origin::Existing,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "pwd",
        run: crate::fs_commands::pwd,
        summary: "Print working directory",
        origin: Origin::Existing,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "read",
        run: crate::files::read,
        summary: "Read numbered lines",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "rg",
        run: crate::search::run_rg,
        summary: "Search files recursively",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "search",
        run: crate::search::run,
        summary: "Search text in files",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "seq",
        run: crate::commands::seq::run,
        summary: "Print numeric sequences",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "sleep",
        run: crate::commands::sleep::run,
        summary: "Delay execution",
        origin: Origin::Ms,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "sort",
        run: crate::commands::sort::run,
        summary: "Sort records",
        origin: Origin::Ms,
        windows_conflict: Some("PowerShell, Windows System32"),
    },
    Utility {
        name: "tac",
        run: crate::commands::tac::run,
        summary: "Concatenate files in reverse",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "tail",
        run: crate::text_commands::tail,
        summary: "Print last lines",
        origin: Origin::Existing,
        windows_conflict: None,
    },
    Utility {
        name: "tee",
        run: crate::commands::tee::run,
        summary: "Copy input to files and stdout",
        origin: Origin::Ms,
        windows_conflict: Some("PowerShell"),
    },
    Utility {
        name: "test",
        run: crate::commands::test::run,
        summary: "Evaluate an expression",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "tr",
        run: crate::commands::tr::run,
        summary: "Translate or delete bytes",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "true",
        run: crate::commands::truefalse::run_true,
        summary: "Exit successfully",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "unexpand",
        run: crate::commands::expand::run_unexpand,
        summary: "Convert spaces to expand",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "uniq",
        run: crate::commands::uniq::run,
        summary: "Filter adjacent repeated records",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "wc",
        run: crate::commands::wc::run,
        summary: "Count input lines, words and bytes",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "xargs",
        run: crate::commands::xargs::run,
        summary: "Build and execute command lines",
        origin: Origin::Ms,
        windows_conflict: None,
    },
    Utility {
        name: "yes",
        run: crate::commands::yes::run,
        summary: "Repeatedly display text",
        origin: Origin::Ms,
        windows_conflict: None,
    },
];

pub fn find(name: &str) -> Option<&'static Utility> {
    UTILITIES
        .binary_search_by_key(&name, |utility| utility.name)
        .ok()
        .map(|index| &UTILITIES[index])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_existing_commands() {
        let expected = [
            "[", "basename", "cat", "cd", "comm", "curl", "cut", "df", "diff", "dirname", "echo",
            "egrep", "env", "expand", "false", "fgrep", "find", "fold", "grep", "head", "less",
            "list", "ls", "nl", "paste", "printenv", "printf", "ps", "pwd", "read", "rg", "search",
            "seq", "sleep", "sort", "tac", "tail", "tee", "test", "tr", "true", "unexpand", "uniq",
            "wc", "xargs", "yes",
        ];
        assert_eq!(
            UTILITIES.iter().map(|u| u.name).collect::<Vec<_>>(),
            expected
        );
        assert!(UTILITIES.windows(2).all(|w| w[0].name < w[1].name));
        assert!(expected.iter().all(|name| find(name).is_some()));
        assert!(find("missing").is_none());
    }
}
