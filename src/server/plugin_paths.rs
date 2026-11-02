//! Resolve plugin paths before taking the server state lock.
use crate::plugins::PanePluginPaths;
use std::cell::RefCell;

thread_local! {
    static PATHS: RefCell<Option<PanePluginPaths>> = const { RefCell::new(None) };
    #[cfg(test)]
    static PREPARE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A request-local snapshot, also shared by all panes restored in one operation.
pub(crate) struct Scope(Option<PanePluginPaths>);

impl Drop for Scope {
    fn drop(&mut self) {
        PATHS.with(|paths| *paths.borrow_mut() = self.0.take());
    }
}

pub(crate) fn prepare() -> Scope {
    #[cfg(test)]
    PREPARE_COUNT.with(|count| count.set(count.get() + 1));
    let paths = crate::plugins::pane_path_dirs().unwrap_or_else(|error| PanePluginPaths {
        dirs: Vec::new(),
        diagnostics: vec![format!("plugin PATH registry: {error:#}")],
    });
    Scope(PATHS.with(|current| current.replace(Some(paths))))
}

#[cfg(test)]
pub(super) fn prepare_count() -> usize {
    PREPARE_COUNT.with(std::cell::Cell::get)
}

pub(super) fn current() -> PanePluginPaths {
    PATHS.with(|paths| {
        let paths = paths.borrow();
        match paths.as_ref() {
            Some(paths) => PanePluginPaths {
                dirs: paths.dirs.clone(),
                diagnostics: paths.diagnostics.clone(),
            },
            None => PanePluginPaths {
                dirs: Vec::new(),
                diagnostics: vec!["plugin PATH snapshot was not prepared".into()],
            },
        }
    })
}

/// Prepend approved directories only when the pane already has a PATH to preserve.
pub(super) fn apply(cmd: &mut portable_pty::CommandBuilder, paths: &mut PanePluginPaths) {
    if paths.dirs.is_empty() {
        return;
    }
    let Some(base) = cmd.get_env("PATH") else {
        paths.dirs.clear();
        paths
            .diagnostics
            .push("cannot prepend plugin directories: PATH is not set".into());
        return;
    };
    match std::env::join_paths(
        paths
            .dirs
            .iter()
            .cloned()
            .chain(std::env::split_paths(base)),
    ) {
        Ok(path) => cmd.env("PATH", path),
        Err(error) => {
            paths.dirs.clear();
            paths.diagnostics.push(format!(
                "cannot prepend plugin directories to PATH: {error}"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_base_path_is_not_replaced_by_plugin_directories() {
        let mut cmd = portable_pty::CommandBuilder::new("test");
        cmd.env_remove("PATH");
        let mut paths = PanePluginPaths {
            dirs: vec![std::env::temp_dir()],
            diagnostics: Vec::new(),
        };
        apply(&mut cmd, &mut paths);
        assert!(cmd.get_env("PATH").is_none());
        assert!(paths.dirs.is_empty());
        assert!(paths.diagnostics[0].contains("PATH is not set"));
    }
}
