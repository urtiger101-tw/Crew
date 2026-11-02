//! Resolve plugin paths before taking the server state lock.
use crate::plugins::PanePluginPaths;
use std::cell::RefCell;

thread_local! {
    static PATHS: RefCell<Option<PanePluginPaths>> = const { RefCell::new(None) };
}

/// A request-local snapshot, also shared by all panes restored in one operation.
pub(crate) struct Scope(Option<PanePluginPaths>);

impl Drop for Scope {
    fn drop(&mut self) {
        PATHS.with(|paths| *paths.borrow_mut() = self.0.take());
    }
}

pub(crate) fn prepare() -> Scope {
    let paths = crate::plugins::pane_path_dirs().unwrap_or_else(|error| PanePluginPaths {
        dirs: Vec::new(),
        diagnostics: vec![format!("plugin PATH registry: {error:#}")],
    });
    Scope(PATHS.with(|current| current.replace(Some(paths))))
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
