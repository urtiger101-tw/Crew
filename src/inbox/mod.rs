//! Durable peer messages. The daemon stores manifests; frontends read sender files.
pub(crate) mod cli;
mod store;

use crate::api::{ApiError, ApiResult, Reply, lock};
use crate::server::Shared;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Handoff,
    Review,
    Question,
    Answer,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Read,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckResult {
    Ok,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: String,
    pub repo_root: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub key: String,
    pub from: String,
    pub to: String,
    pub kind: Kind,
    pub reply_to: Option<String>,
    pub summary: String,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ack {
    pub stage: Stage,
    pub by: String,
    pub at: String,
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AckResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Message {
    pub id: String,
    #[serde(flatten)]
    pub content: Submission,
    pub created_at: String,
    pub acks: Vec<Ack>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Peers,
    Send {
        caller: Option<usize>,
        message: Submission,
    },
    List {
        to: String,
        open: bool,
        unread: bool,
        after: Option<String>,
        limit: usize,
    },
    Read {
        id: String,
    },
    Ack {
        id: String,
        by: String,
        stage: Stage,
        result: Option<AckResult>,
        note: Option<String>,
    },
}

pub(crate) fn valid_artifact_path(path: &str) -> bool {
    !path.contains(['\\', ':', '\0'])
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

pub(crate) fn validate(req: &Request) -> Result<(), ApiError> {
    match req {
        Request::Send { message: m, .. } => {
            if !crate::api::is_agent_name(&m.from) || !crate::api::is_agent_name(&m.to) {
                return Err(ApiError::invalid("from/to must be named peers"));
            }
            if m.key.is_empty()
                || m.key.len() > 128
                || !m
                    .key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            {
                return Err(ApiError::invalid(
                    "key must be 1..128 ASCII bytes from A-Z a-z 0-9 . _ : -",
                ));
            }
            if m.summary.len() > 512 || m.artifacts.len() > 16 {
                return Err(ApiError::invalid(
                    "summary exceeds 512 bytes or artifacts exceed 16",
                ));
            }
            if m.reply_to.as_ref().is_some_and(|id| id.len() > 1024) {
                return Err(ApiError::invalid("reply_to exceeds 1024 UTF-8 bytes"));
            }
            for a in &m.artifacts {
                if !valid_artifact_path(&a.path)
                    || a.path.len() > 1024
                    || a.repo_root.len() > 1024
                    || !std::path::Path::new(&a.repo_root).is_absolute()
                    || a.sha256.len() != 64
                    || !a
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(ApiError::invalid(
                        "invalid artifact path, absolute repo_root, or SHA-256; path/root limit is 1024 UTF-8 bytes",
                    ));
                }
            }
        }
        Request::List {
            to,
            open,
            unread,
            limit,
            ..
        } => {
            if !crate::api::is_agent_name(to) || (*open && *unread) || !(1..=100).contains(limit) {
                return Err(ApiError::invalid(
                    "list needs a named recipient, limit 1..100, and at most one of --open/--unread",
                ));
            }
        }
        Request::Ack {
            by,
            stage,
            result,
            note,
            ..
        } => {
            if !crate::api::is_agent_name(by)
                || note.as_ref().is_some_and(|n| n.len() > 512)
                || matches!(
                    (stage, result),
                    (Stage::Read, Some(_)) | (Stage::Done, None)
                )
            {
                return Err(ApiError::invalid(
                    "ack needs a named receiver, note <=512 bytes, and a result only for done",
                ));
            }
        }
        Request::Peers | Request::Read { .. } => {}
    }
    Ok(())
}

pub(crate) fn handle(shared: &Shared, req: Request) -> ApiResult {
    let known = {
        let st = lock(shared);
        if matches!(req, Request::Peers) {
            let peers: Vec<_> = st.panes.iter().filter_map(|(id, p)| {
                let a = p.agent.as_ref()?;
                // Restored instance 0 is a saved placeholder, pending a live report.
                if a.instance == 0 { return None; }
                let name = a.name.as_ref()?;
                Some(json!({"name":name,"pane_id":crate::api::pane_id_str(*id),"instance":a.instance,"source":a.source,"agent":a.agent,"state":a.state}))
            }).collect();
            return Ok(Reply::Json(
                json!({"epoch":st.events.epoch(),"peers":peers}),
            ));
        }
        if let Request::Send { caller, message } = &req {
            let caller = caller.ok_or_else(|| ApiError::invalid("send requires CREW_PANE_ID"))?;
            let name = st
                .panes
                .get(&caller)
                .and_then(|p| p.agent.as_ref())
                .filter(|a| a.instance != 0)
                .and_then(|a| a.name.as_deref())
                .ok_or_else(|| {
                    ApiError::new("agent_not_found", "caller has no live named reported agent")
                })?;
            if name != message.from {
                return Err(ApiError::invalid(
                    "--from does not match caller's live reported name",
                ));
            }
        }
        st.panes
            .values()
            .filter_map(|p| p.agent.as_ref().filter(|a| a.instance != 0)?.name.clone())
            .collect::<Vec<_>>()
    };
    validate(&req)?;
    let (value, event) = store::execute(&store_path()?, &req, &known)?;
    if let Some((kind, data)) = event {
        lock(shared).events.record(kind, None, data);
    }
    Ok(Reply::Json(value))
}

fn store_path() -> Result<std::path::PathBuf, ApiError> {
    let override_path = crate::brand::env_os("INBOX_STORE").map(Into::into);
    if override_path.is_some() {
        return resolve_store_path(None, "", override_path);
    }
    resolve_store_path(crate::persist::state_path(), &crate::ipc::base_name(), None)
}

fn resolve_store_path(
    state: Option<std::path::PathBuf>,
    socket: &str,
    override_path: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, ApiError> {
    if let Some(path) = override_path {
        if !path.is_absolute() || path.file_name().is_none() {
            return Err(ApiError::invalid(
                "CREW_INBOX_STORE must be an absolute file path",
            ));
        }
        return Ok(path);
    }
    let stem = socket
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    if socket.is_empty()
        || matches!(socket, "." | "..")
        || socket.ends_with(['.', ' '])
        || reserved
        || socket
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
    {
        return Err(ApiError::invalid(
            "socket name needs CREW_INBOX_STORE: not a single filename component",
        ));
    }
    let state = state.ok_or_else(|| ApiError::new("internal", "cannot resolve Crew state file"))?;
    let state = if state.is_absolute() {
        state
    } else {
        std::env::current_dir()
            .map_err(|e| ApiError::new("internal", e.to_string()))?
            .join(state)
    };
    Ok(state
        .parent()
        .ok_or_else(|| ApiError::invalid("state file has no parent"))?
        .join("inbox")
        .join(format!("{socket}.json")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn frozen_note_and_artifact_limits() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../docs/collab/claude-codex/contract/b-v1.fixtures.json"
        ))
        .unwrap();
        for b in fixture["note_boundaries"].as_array().unwrap() {
            let req = Request::Ack {
                id: "fixture".into(),
                by: "crew-claude".into(),
                stage: Stage::Read,
                result: None,
                note: b["text"].as_str().map(Into::into),
            };
            assert_eq!(
                validate(&req).is_ok(),
                b["accepted"].as_bool().unwrap(),
                "{}",
                b["id"]
            );
        }
        for b in fixture["artifact_count_boundaries"].as_array().unwrap() {
            let mut m: Submission = serde_json::from_value({
                let mut value = fixture["messages"]["stored"].clone();
                let object = value.as_object_mut().unwrap();
                for key in ["id", "created_at", "acks"] {
                    object.remove(key);
                }
                value
            })
            .unwrap();
            let mut artifact: Artifact =
                serde_json::from_value(b["artifact_template"].clone()).unwrap();
            artifact.repo_root = std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            m.artifacts = vec![artifact; b["count"].as_u64().unwrap() as usize];
            assert_eq!(
                validate(&Request::Send {
                    caller: Some(4),
                    message: m
                })
                .is_ok(),
                b["accepted"].as_bool().unwrap()
            );
        }
    }

    #[test]
    fn frozen_store_paths_and_override() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../docs/collab/claude-codex/contract/b-v1.fixtures.json"
        ))
        .unwrap();
        for b in fixture["store_examples"].as_array().unwrap() {
            // The frozen path examples use Windows absolute paths.
            if !cfg!(windows) {
                continue;
            }
            let result = resolve_store_path(
                b["crew_state_file"].as_str().map(Into::into),
                b["socket_name"].as_str().unwrap(),
                b["inbox_override"].as_str().map(Into::into),
            );
            if let Some(expected) = b["expected_path"].as_str() {
                assert_eq!(result.unwrap(), std::path::Path::new(expected));
            } else {
                assert_eq!(result.unwrap_err().code, b["error"].as_str().unwrap());
            }
        }
        assert!(resolve_store_path(None, "CON.sock", None).is_err());
        for socket in [
            "con .sock",
            "NUL  .json",
            "CONIN$.sock",
            "conout$.sock",
            "com1 .sock",
            "LPT9  .sock",
        ] {
            assert_eq!(
                resolve_store_path(Some("state.json".into()), socket, None)
                    .unwrap_err()
                    .code,
                "invalid_argument",
                "{socket}"
            );
        }
        assert!(resolve_store_path(None, "crew.sock", Some("relative.json".into())).is_err());
    }

    #[test]
    fn daemon_manifest_string_limits_count_utf8_bytes() {
        let root = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let original = Submission {
            key: "limits".into(),
            from: "crew-codex".into(),
            to: "crew-claude".into(),
            kind: Kind::Review,
            reply_to: None,
            summary: "Review".into(),
            artifacts: vec![Artifact {
                path: "file.txt".into(),
                repo_root: root.clone(),
                sha256: "a".repeat(64),
                bytes: 1,
            }],
        };
        for field in ["path", "repo_root", "reply_to"] {
            for length in [1024, 1025] {
                let mut message = original.clone();
                match field {
                    "path" => {
                        message.artifacts[0].path =
                            format!("{}{}", "中".repeat(341), "x".repeat(length - 1023))
                    }
                    "repo_root" => {
                        message.artifacts[0].repo_root =
                            format!("{root}/{}", "x".repeat(length - root.len() - 1))
                    }
                    _ => message.reply_to = Some("r".repeat(length)),
                }
                let result = validate(&Request::Send {
                    caller: Some(4),
                    message,
                });
                assert_eq!(result.is_ok(), length == 1024, "{field}: {length}");
                if let Err(error) = result {
                    assert_eq!(error.code, "invalid_argument");
                }
            }
        }
    }
}
