//! The CLI and JSON-lines bridge share this frontend, including artifact manifests.
use super::{AckResult, Artifact, Kind, Request, Stage, Submission, valid_artifact_path};
use crate::api::Request as ApiRequest;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub(crate) const HELP: &str = "\
crew peer list
crew msg send --from NAME --to NAME --key KEY --kind handoff|review|question|answer|done --summary TEXT [--reply-to ID] [--repo-root ABSOLUTE_PATH --file RELATIVE_PATH ...]
crew msg list --to NAME [--open | --unread] [--after CURSOR] [--limit 1..100]
crew msg read ID
crew msg ack ID --by NAME --stage read [--note TEXT]
crew msg ack ID --by NAME --stage done --result ok|failed [--note TEXT]

Send uses CREW_PANE_ID to check the caller's live reported peer name.
Artifact paths use / and stay within the explicit --repo-root; the frontend records SHA-256 and byte length.
Reading a message does not acknowledge it. Done requires an earlier read acknowledgement.
--open includes read messages that are not done. Inbox cursors survive daemon restarts.
CREW_INBOX_STORE overrides the per-socket message store with an absolute file path.
";

pub(crate) fn parse(
    group: &str,
    sub: &str,
    args: &[String],
    current: Option<usize>,
) -> Result<ApiRequest, String> {
    let (values, flags): (&[&str], &[&str]) = match (group, sub) {
        ("peer", "list") => (&[], &[]),
        ("msg", "send") => (
            &[
                "from",
                "to",
                "key",
                "kind",
                "reply-to",
                "summary",
                "repo-root",
                "file",
            ],
            &[],
        ),
        ("msg", "list") => (&["to", "after", "limit"], &["open", "unread"]),
        ("msg", "read") => (&[], &[]),
        ("msg", "ack") => (&["by", "stage", "result", "note"], &[]),
        _ => return Err(format!("unknown command: {group} {sub}")),
    };
    let mut options = BTreeMap::new();
    let mut files = Vec::new();
    let mut positional = Vec::new();
    let mut index = 0;
    // Finish argument scanning before opening any artifact. Help therefore has no I/O
    // side effects, while a help-looking word consumed as a value stays literal text.
    while let Some(arg) = args.get(index) {
        index += 1;
        if arg == "--" {
            positional.extend_from_slice(&args[index..]);
            break;
        }
        if crate::api::cli::is_help_flag(arg) {
            return Err(crate::api::cli::HELP.into());
        }
        let Some(body) = arg.strip_prefix("--") else {
            if arg.starts_with('-') && arg.len() > 1 {
                return Err(format!("unknown option: {arg}"));
            }
            positional.push(arg.clone());
            continue;
        };
        let (name, inline) = body
            .split_once('=')
            .map_or((body, None), |(name, value)| (name, Some(value)));
        let value = if values.contains(&name) {
            if let Some(value) = inline {
                value.to_string()
            } else {
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("missing value for --{name}"))?;
                index += 1;
                value.clone()
            }
        } else if flags.contains(&name) {
            if inline.is_some() {
                return Err(format!("option --{name} takes no value"));
            }
            String::new()
        } else {
            return Err(format!("unknown option: --{name}"));
        };
        if name == "file" {
            files.push(value);
        } else if options.insert(name.to_string(), value).is_some() {
            return Err(format!("duplicate option: --{name}"));
        }
    }
    let expected = usize::from(matches!((group, sub), ("msg", "read" | "ack")));
    if positional.len() != expected {
        return Err(format!(
            "{group} {sub} expects {expected} positional argument(s)"
        ));
    }
    let required = |name: &str| {
        options
            .get(name)
            .cloned()
            .ok_or_else(|| format!("missing --{name}"))
    };
    let request = match (group, sub) {
        ("peer", "list") => Request::Peers,
        ("msg", "send") => {
            let mut message = Submission {
                key: required("key")?,
                from: required("from")?,
                to: required("to")?,
                kind: match required("kind")?.as_str() {
                    "handoff" => Kind::Handoff,
                    "review" => Kind::Review,
                    "question" => Kind::Question,
                    "answer" => Kind::Answer,
                    "done" => Kind::Done,
                    _ => {
                        return Err(
                            "--kind must be handoff, review, question, answer, or done".into()
                        );
                    }
                },
                reply_to: options.get("reply-to").cloned(),
                summary: required("summary")?,
                artifacts: Vec::new(),
            };
            if !files.is_empty() {
                let repo_root = required("repo-root")?;
                if !Path::new(&repo_root).is_absolute() {
                    return Err("--repo-root must be an explicit absolute path".into());
                }
                if files.len() > 16 {
                    return Err("at most 16 --file arguments are allowed".into());
                }
                for path in &files {
                    if !valid_artifact_path(path) {
                        return Err(format!("invalid repository-relative artifact path: {path}"));
                    }
                }
                let root = Path::new(&repo_root)
                    .canonicalize()
                    .map_err(|e| format!("cannot resolve --repo-root: {e}"))?;
                for path in files {
                    let resolved = root
                        .join(&path)
                        .canonicalize()
                        .map_err(|e| format!("cannot resolve artifact {path}: {e}"))?;
                    if !resolved.starts_with(&root) {
                        return Err(format!("artifact escapes --repo-root: {path}"));
                    }
                    if !resolved
                        .metadata()
                        .map_err(|e| format!("cannot inspect artifact {path}: {e}"))?
                        .is_file()
                    {
                        return Err(format!("artifact must be a regular file: {path}"));
                    }
                    let mut file = File::open(&resolved)
                        .map_err(|e| format!("cannot open artifact {path}: {e}"))?;
                    let mut digest = Sha256::new();
                    let mut bytes = 0;
                    let mut buffer = [0u8; 64 * 1024];
                    loop {
                        let count = file
                            .read(&mut buffer)
                            .map_err(|e| format!("cannot read artifact {path}: {e}"))?;
                        if count == 0 {
                            break;
                        }
                        digest.update(&buffer[..count]);
                        bytes += count as u64;
                    }
                    message.artifacts.push(Artifact {
                        path,
                        repo_root: repo_root.clone(),
                        sha256: format!("{:x}", digest.finalize()),
                        bytes,
                    });
                }
            }
            Request::Send {
                caller: current,
                message,
            }
        }
        ("msg", "list") => Request::List {
            to: required("to")?,
            open: options.contains_key("open"),
            unread: options.contains_key("unread"),
            after: options.get("after").cloned(),
            limit: options
                .get("limit")
                .map(|value| value.parse().map_err(|_| "--limit must be an integer"))
                .transpose()?
                .unwrap_or(20),
        },
        ("msg", "read") => Request::Read {
            id: positional.remove(0),
        },
        ("msg", "ack") => Request::Ack {
            id: positional.remove(0),
            by: required("by")?,
            stage: match required("stage")?.as_str() {
                "read" => Stage::Read,
                "done" => Stage::Done,
                _ => return Err("--stage must be read or done".into()),
            },
            result: options
                .get("result")
                .map(|value| match value.as_str() {
                    "ok" => Ok(AckResult::Ok),
                    "failed" => Ok(AckResult::Failed),
                    _ => Err("--result must be ok or failed"),
                })
                .transpose()?,
            note: options.get("note").cloned(),
        },
        _ => unreachable!("command validated before scanning"),
    };
    Ok(ApiRequest::Inbox(request))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).into()).collect()
    }

    fn send() -> Vec<String> {
        args(&[
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "review-b-1",
            "--kind",
            "review",
            "--summary",
            "Review",
        ])
    }

    #[test]
    fn help_scans_before_artifact_io_and_respects_values() {
        let mut words = send();
        words.extend(args(&[
            "--file",
            "missing.txt",
            "--repo-root",
            "missing-root",
            "--help",
        ]));
        assert_eq!(
            parse("msg", "send", &words, Some(4)),
            Err(crate::api::cli::HELP.into())
        );
        for value in ["-h", "--help"] {
            let mut words = send();
            *words.last_mut().unwrap() = value.into();
            let ApiRequest::Inbox(Request::Send { caller, message }) =
                parse("msg", "send", &words, Some(4)).unwrap()
            else {
                panic!("send expected")
            };
            assert_eq!(caller, Some(4));
            assert_eq!(message.summary, value);
        }
        let read = parse("msg", "read", &args(&["--", "--help"]), None).unwrap();
        assert_eq!(
            read,
            ApiRequest::Inbox(Request::Read {
                id: "--help".into()
            })
        );
    }

    #[test]
    fn artifact_directories_are_rejected_before_open() {
        let mut words = send();
        words.extend(args(&[
            "--repo-root",
            env!("CARGO_MANIFEST_DIR"),
            "--file",
            "src/inbox",
        ]));
        assert!(
            parse("msg", "send", &words, Some(4))
                .unwrap_err()
                .contains("must be a regular file")
        );
    }

    #[test]
    fn frozen_artifact_manifest_is_hashed_by_frontend() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/docs/collab/claude-codex/contract/b-v1.fixtures.json"
        )))
        .unwrap();
        let expected = &fixture["messages"]["stored"]["artifacts"][0];
        let mut words = send();
        words.extend(args(&[
            "--repo-root",
            env!("CARGO_MANIFEST_DIR"),
            "--file",
            expected["path"].as_str().unwrap(),
        ]));
        let ApiRequest::Inbox(Request::Send { message, .. }) =
            parse("msg", "send", &words, Some(4)).unwrap()
        else {
            panic!("send expected")
        };
        assert_eq!(message.artifacts[0].sha256, expected["sha256"]);
        assert_eq!(
            message.artifacts[0].bytes,
            expected["bytes"].as_u64().unwrap()
        );
        assert_eq!(message.artifacts[0].repo_root, env!("CARGO_MANIFEST_DIR"));
        for invalid in fixture["invalid_artifact_paths"].as_array().unwrap() {
            *words.last_mut().unwrap() = invalid.as_str().unwrap().into();
            assert!(
                parse("msg", "send", &words, Some(4))
                    .unwrap_err()
                    .starts_with("invalid repository-relative artifact path:")
            );
        }
    }

    #[test]
    fn parser_preserves_boundary_values_for_daemon_validation() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/docs/collab/claude-codex/contract/b-v1.fixtures.json"
        )))
        .unwrap();
        for boundary in fixture["summary_boundaries"].as_array().unwrap() {
            let mut words = send();
            *words.last_mut().unwrap() = boundary["text"].as_str().unwrap().into();
            let ApiRequest::Inbox(request) = parse("msg", "send", &words, Some(4)).unwrap() else {
                panic!("inbox expected")
            };
            assert_eq!(
                super::super::validate(&request).is_ok(),
                boundary["accepted"].as_bool().unwrap()
            );
        }
        for boundary in fixture["key_boundaries"].as_array().unwrap() {
            let mut words = send();
            words[5] = boundary["text"].as_str().unwrap().into();
            let ApiRequest::Inbox(request) = parse("msg", "send", &words, Some(4)).unwrap() else {
                panic!("inbox expected")
            };
            assert_eq!(
                super::super::validate(&request).is_ok(),
                boundary["accepted"].as_bool().unwrap()
            );
        }
        assert!(matches!(
            parse("msg", "list", &args(&["--to", "crew-claude"]), None),
            Ok(ApiRequest::Inbox(Request::List { limit: 20, .. }))
        ));
        let ApiRequest::Inbox(request) = parse(
            "msg",
            "list",
            &args(&["--to", "crew-claude", "--open", "--unread"]),
            None,
        )
        .unwrap() else {
            panic!("inbox expected")
        };
        assert!(super::super::validate(&request).is_err());
    }

    #[test]
    fn rejects_unknown_duplicates_missing_values_and_extra_positionals() {
        assert!(parse("msg", "send", &args(&["--unknown", "x"]), None).is_err());
        assert!(parse("msg", "list", &args(&["--to", "a", "--to", "b"]), None).is_err());
        assert!(parse("msg", "list", &args(&["--to"]), None).is_err());
        assert!(parse("peer", "list", &args(&["extra"]), None).is_err());
        assert!(parse("msg", "read", &[], None).is_err());
        let mut words = send();
        words.extend(args(&["--file", "missing.txt"]));
        assert_eq!(
            parse("msg", "send", &words, None).unwrap_err(),
            "missing --repo-root"
        );
    }

    #[test]
    fn ack_cli_keeps_explicit_stage_result_and_note() {
        let actual = parse(
            "msg",
            "ack",
            &args(&[
                "m1",
                "--by",
                "crew-claude",
                "--stage",
                "done",
                "--result",
                "failed",
                "--note",
                "-h",
            ]),
            None,
        )
        .unwrap();
        assert_eq!(
            actual,
            ApiRequest::Inbox(Request::Ack {
                id: "m1".into(),
                by: "crew-claude".into(),
                stage: Stage::Done,
                result: Some(AckResult::Failed),
                note: Some("-h".into()),
            })
        );
    }
}
