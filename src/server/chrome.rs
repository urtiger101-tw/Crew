//! Read-only Aurora context. Filesystem and Git work never holds the UI lock.
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub status: String,
    pub path: String,
}

#[derive(Default)]
pub struct Snapshot {
    pub files: BTreeMap<String, Result<Vec<ChangedFile>, String>>,
    pub inbox: BTreeMap<String, usize>,
    pub inbox_error: Option<String>,
}

#[derive(Default)]
pub struct Cache {
    pub snapshot: Snapshot,
    pending: Option<Receiver<Snapshot>>,
    requested: Vec<String>,
    last_request: Option<Instant>,
}

impl Cache {
    /// Called on UI events and render ticks; at most one worker is active.
    pub fn refresh(&mut self, directories: Vec<String>) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(snapshot) => {
                    self.snapshot = snapshot;
                    self.pending = None;
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    self.snapshot.inbox_error = Some("context worker stopped".into());
                    self.pending = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.pending.is_some()
            || (self.requested == directories
                && self
                    .last_request
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(2)))
        {
            return changed;
        }
        self.requested = directories.clone();
        self.last_request = Some(Instant::now());
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let mut snapshot = Snapshot::default();
            match crate::inbox::open_counts() {
                Ok(counts) => snapshot.inbox = counts,
                Err(error) => snapshot.inbox_error = Some(error.message),
            }
            for cwd in directories {
                let mut cmd = Command::new("git");
                cmd.args([
                    "-C",
                    &cwd,
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--untracked-files=normal",
                ])
                .env("GIT_OPTIONAL_LOCKS", "0");
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                }
                let files = cmd.output().map_err(|e| e.to_string()).and_then(|out| {
                    if out.status.success() {
                        Ok(parse_status(&out.stdout))
                    } else {
                        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
                    }
                });
                snapshot.files.insert(cwd, files);
            }
            let _ = tx.send(snapshot);
        });
        changed
    }
}

fn parse_status(bytes: &[u8]) -> Vec<ChangedFile> {
    let mut paths = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty());
    let mut files = Vec::new();
    while let Some(record) = paths.next() {
        if record.len() < 4 {
            continue;
        }
        let status = String::from_utf8_lossy(&record[..2]).into_owned();
        let mut path = String::from_utf8_lossy(&record[3..]).into_owned();
        if status.contains(['R', 'C'])
            && let Some(from) = paths.next()
        {
            path = format!("{} -> {path}", String::from_utf8_lossy(from));
        }
        // Filenames are data; control bytes never become terminal instructions.
        let path = path
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        files.push(ChangedFile { status, path });
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_status_keeps_spaces_and_consumes_rename_source() {
        let files =
            parse_status(b" M has space.rs\0R  next.rs\0old.rs\0?? folder/\0 M line\nname\0");
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].path, "has space.rs");
        assert_eq!(files[1].path, "old.rs -> next.rs");
        assert_eq!(files[2].status, "??");
        assert_eq!(files[3].path, "line name");
    }
}
