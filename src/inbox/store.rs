use super::{Ack, Message, Request, Stage, Submission};
use crate::api::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_MESSAGES: usize = 10_000;
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub(super) type Outcome = (Value, Option<(&'static str, Value)>);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    identity: String,
    messages: Vec<Message>,
}

impl Store {
    fn empty() -> Self {
        Self {
            version: 1,
            identity: fresh_token(),
            messages: Vec::new(),
        }
    }

    fn cursor(&self, index: usize) -> String {
        format!("inbox:{}:{}", self.identity, index + 1)
    }
}

fn fresh_token() -> String {
    format!(
        "{:x}-{:x}-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    ApiError::new("internal", format!("inbox store: {error}"))
}

fn timestamp() -> Result<String, ApiError> {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(internal)?;
    Ok(rfc3339_utc(since.as_secs(), since.subsec_millis()))
}

/// RFC 3339 UTC text (`YYYY-MM-DDTHH:MM:SS.mmmZ`) for a Unix time, without a date crate.
fn rfc3339_utc(secs: u64, millis: u32) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Proleptic Gregorian date for days since 1970-01-01 (Howard Hinnant's civil_from_days).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod time_tests {
    use super::rfc3339_utc;

    #[test]
    fn rfc3339_utc_matches_known_instants() {
        assert_eq!(rfc3339_utc(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_utc(951_782_400, 7), "2000-02-29T00:00:00.007Z");
        assert_eq!(rfc3339_utc(1_709_251_199, 999), "2024-02-29T23:59:59.999Z");
        assert_eq!(rfc3339_utc(1_793_585_565, 123), "2026-11-02T02:12:45.123Z");
        assert_eq!(rfc3339_utc(4_102_444_800, 0), "2100-01-01T00:00:00.000Z");
    }
}

fn read(path: &Path) -> Result<Store, ApiError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Store::empty()),
        Err(error) => return Err(internal(error)),
    };
    let store: Store = serde_json::from_reader(BufReader::new(file)).map_err(internal)?;
    if store.version != 1 || store.identity.is_empty() || store.messages.len() > MAX_MESSAGES {
        return Err(internal("unsupported or oversized store"));
    }
    Ok(store)
}

fn save(path: &Path, store: &Store) -> Result<(), ApiError> {
    let parent = path
        .parent()
        .ok_or_else(|| internal("file has no parent"))?;
    // Separate staging names by store. Unclaimed old files are retained until a
    // cleanup ownership contract can distinguish them from configured stores.
    let name = path
        .file_name()
        .ok_or_else(|| internal("file has no name"))?;
    let prefix = format!(
        ".crew-inbox-write-{:x}-",
        Sha256::digest(name.as_encoded_bytes())
    );
    let temp = parent.join(format!("{prefix}{}", fresh_token()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(internal)?;
    let result = (|| {
        serde_json::to_writer(&mut file, store).map_err(internal)?;
        file.write_all(b"\n").map_err(internal)?;
        file.sync_all().map_err(internal)?;
        drop(file);
        fs::rename(&temp, path).map_err(internal)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn acquire_lock(file: &File, timeout: Duration) -> Result<(), ApiError> {
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(ApiError::new(
                        "timeout",
                        format!(
                            "inbox store lock timed out after {} ms; retry the request",
                            timeout.as_millis()
                        ),
                    ));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            Err(error) => return Err(internal(error)),
        }
    }
}

fn same_content(a: &Submission, b: &Submission) -> bool {
    a.key == b.key
        && a.from == b.from
        && a.to == b.to
        && a.kind == b.kind
        && a.reply_to == b.reply_to
        && a.summary == b.summary
        && a.artifacts.len() == b.artifacts.len()
        && a.artifacts
            .iter()
            .zip(&b.artifacts)
            .all(|(a, b)| a.path == b.path && a.sha256 == b.sha256 && a.bytes == b.bytes)
}

/// The caller validates request fields and live sender identity before entering this
/// transaction. The sidecar survives atomic replacement, so all readers and writers
/// serialize on the same lock throughout read/check/save.
pub(super) fn execute(path: &Path, req: &Request, known: &[String]) -> Result<Outcome, ApiError> {
    let parent = path
        .parent()
        .ok_or_else(|| internal("file has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| internal("file has no name"))?;
    fs::create_dir_all(parent).map_err(internal)?;
    let mut lock_name = name.to_os_string();
    lock_name.push(".lock");
    let held_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(lock_name))
        .map_err(internal)?;
    acquire_lock(&held_lock, LOCK_TIMEOUT)?;
    let mut store = read(path)?;
    match req {
        Request::Send { message, .. } => {
            if let Some(previous) = store
                .messages
                .iter()
                .find(|m| m.content.from == message.from && m.content.key == message.key)
            {
                return if same_content(&previous.content, message) {
                    Ok((json!({"message":previous,"deduplicated":true}), None))
                } else {
                    Err(ApiError::new(
                        "message_conflict",
                        "same sender/key has different content",
                    ))
                };
            }
            if !known.contains(&message.to)
                && !store
                    .messages
                    .iter()
                    .any(|m| m.content.from == message.to || m.content.to == message.to)
            {
                return Err(ApiError::new(
                    "unknown_peer",
                    "recipient is not a live or historically known named peer",
                ));
            }
            if store.messages.len() == MAX_MESSAGES {
                return Err(ApiError::new(
                    "store_full",
                    "inbox store contains 10000 messages",
                ));
            }
            if message
                .reply_to
                .as_ref()
                .is_some_and(|id| !store.messages.iter().any(|m| &m.id == id))
            {
                return Err(ApiError::invalid(
                    "reply_to must reference a message in this store",
                ));
            }
            let message = Message {
                id: format!("msg_{}_{}", store.identity, store.messages.len() + 1),
                content: message.clone(),
                created_at: timestamp()?,
                acks: Vec::new(),
            };
            let event = json!({"message_id":message.id,"from":message.content.from,"to":message.content.to});
            let response = json!({"message":message,"deduplicated":false});
            store.messages.push(message);
            save(path, &store)?;
            Ok((response, Some(("message.stored", event))))
        }
        Request::Read { id } => {
            let message = store
                .messages
                .iter()
                .find(|m| &m.id == id)
                .ok_or_else(|| ApiError::new("message_not_found", "message not found"))?;
            Ok((json!({"message":message}), None))
        }
        Request::Ack {
            id,
            by,
            stage,
            result,
            note,
        } => {
            let message = store
                .messages
                .iter_mut()
                .find(|m| &m.id == id)
                .ok_or_else(|| ApiError::new("message_not_found", "message not found"))?;
            if &message.content.to != by {
                return Err(ApiError::invalid(
                    "ack by must equal the intended recipient",
                ));
            }
            if let Some(previous) = message.acks.iter().find(|a| a.stage == *stage) {
                return if previous.by == *by && previous.result == *result && previous.note == *note
                {
                    Ok((json!({"message":message,"changed":false}), None))
                } else {
                    Err(ApiError::new(
                        "ack_conflict",
                        "existing ack stage has different content",
                    ))
                };
            }
            if *stage == Stage::Done && !message.acks.iter().any(|a| a.stage == Stage::Read) {
                return Err(ApiError::invalid(
                    "read acknowledgement is required before done",
                ));
            }
            message.acks.push(Ack {
                stage: *stage,
                by: by.clone(),
                at: timestamp()?,
                note: note.clone(),
                result: *result,
            });
            let mut event = json!({"message_id":id,"stage":stage,"by":by});
            if *stage == Stage::Done {
                event["result"] = json!(result);
            }
            let response = json!({"message":message,"changed":true});
            save(path, &store)?;
            Ok((response, Some(("message.acked", event))))
        }
        Request::List {
            to,
            open,
            unread,
            after,
            limit,
        } => {
            let start = match after {
                None => 0,
                Some(cursor) => {
                    let prefix = format!("inbox:{}:", store.identity);
                    cursor
                        .strip_prefix(&prefix)
                        .and_then(|s| s.parse::<usize>().ok())
                        .filter(|n| {
                            *n > 0
                                && *n <= store.messages.len()
                                && cursor == &format!("{prefix}{n}")
                        })
                        .ok_or_else(|| {
                            ApiError::invalid(
                                "invalid cursor or cursor belongs to another inbox store",
                            )
                        })?
                }
            };
            let matching: Vec<_> = store
                .messages
                .iter()
                .enumerate()
                .skip(start)
                .filter(|(_, m)| {
                    m.content.to == *to
                        && (!*open || !m.acks.iter().any(|a| a.stage == Stage::Done))
                        && (!*unread || !m.acks.iter().any(|a| a.stage == Stage::Read))
                })
                .take(limit + 1)
                .collect();
            let has_more = matching.len() > *limit;
            let page = &matching[..matching.len().min(*limit)];
            let next_cursor = page
                .last()
                .map(|(i, _)| store.cursor(*i))
                .or_else(|| after.clone());
            let messages: Vec<_> = page.iter().map(|(_, m)| json!({
                "id":m.id,"from":m.content.from,"to":m.content.to,"kind":m.content.kind,
                "reply_to":m.content.reply_to,"summary":m.content.summary,"created_at":m.created_at,
                "read":m.acks.iter().any(|a| a.stage == Stage::Read),
                "done":m.acks.iter().find(|a| a.stage == Stage::Done).and_then(|a| a.result),
            })).collect();
            Ok((
                json!({"messages":messages,"next_cursor":next_cursor,"has_more":has_more}),
                None,
            ))
        }
        Request::Peers => Err(internal("peer projection is not a store operation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::{AckResult, Kind};
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("crew-inbox-store-{}", fresh_token()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn path(&self) -> PathBuf {
            self.0.join("inbox.json")
        }
        fn run(&self, req: &Request) -> Result<Outcome, ApiError> {
            execute(
                &self.path(),
                req,
                &["crew-codex".into(), "crew-claude".into()],
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn fixtures() -> Value {
        serde_json::from_str(include_str!(
            "../../docs/collab/claude-codex/contract/b-v1.fixtures.json"
        ))
        .unwrap()
    }
    fn fixture_message(name: &str) -> Message {
        serde_json::from_value(fixtures()["messages"][name].clone()).unwrap()
    }
    fn send(key: &str) -> Request {
        let mut message = fixture_message("stored").content;
        message.key = key.into();
        Request::Send {
            caller: Some(4),
            message,
        }
    }
    fn ack(id: &str, stage: Stage, result: Option<AckResult>, note: Option<&str>) -> Request {
        Request::Ack {
            id: id.into(),
            by: "crew-claude".into(),
            stage,
            result,
            note: note.map(str::to_owned),
        }
    }
    fn list(open: bool, unread: bool, after: Option<String>, limit: usize) -> Request {
        Request::List {
            to: "crew-claude".into(),
            open,
            unread,
            after,
            limit,
        }
    }

    #[test]
    fn frozen_message_fixtures_roundtrip() {
        for expected in fixtures()["messages"].as_object().unwrap().values() {
            let message: Message = serde_json::from_value(expected.clone()).unwrap();
            assert_eq!(serde_json::to_value(message).unwrap(), *expected);
        }
    }

    #[test]
    fn retry_excludes_root_preserves_original_and_current_acks() {
        let f = Fixture::new();
        let (stored, event) = f.run(&send("review-b-1")).unwrap();
        assert_eq!(event.unwrap().0, "message.stored");
        let id = stored["message"]["id"].as_str().unwrap();
        let (acked, event) = f
            .run(&ack(
                id,
                Stage::Read,
                None,
                Some("Receiver fetched the message"),
            ))
            .unwrap();
        assert_eq!(
            event.unwrap().1,
            json!({"message_id":id,"stage":"read","by":"crew-claude"})
        );
        let mut retry = send("review-b-1");
        let Request::Send { message, .. } = &mut retry else {
            unreachable!()
        };
        message.artifacts[0].repo_root = "D:/OtherWorktree".into();
        let (result, event) = execute(&f.path(), &retry, &[]).unwrap();
        assert_eq!(result["message"], acked["message"]);
        assert_eq!(result["deduplicated"], true);
        assert!(event.is_none());
        let before = fs::read(f.path()).unwrap();
        let Request::Send { message, .. } = &mut retry else {
            unreachable!()
        };
        message.artifacts[0].bytes += 1;
        assert_eq!(f.run(&retry).unwrap_err().code, "message_conflict");
        assert_eq!(fs::read(f.path()).unwrap(), before);
    }

    #[test]
    fn ack_order_retry_conflict_and_terminal_failed() {
        let f = Fixture::new();
        let (stored, _) = f.run(&send("acks")).unwrap();
        let id = stored["message"]["id"].as_str().unwrap();
        let done = ack(
            id,
            Stage::Done,
            Some(AckResult::Failed),
            Some("Artifact mismatch; no sender-root fallback"),
        );
        assert_eq!(f.run(&done).unwrap_err().code, "invalid_argument");
        let read_req = ack(id, Stage::Read, None, None);
        let (read, _) = f.run(&read_req).unwrap();
        let (finished, event) = f.run(&done).unwrap();
        assert_eq!(
            event.unwrap().1,
            json!({"message_id":id,"stage":"done","by":"crew-claude","result":"failed"})
        );
        let (again, event) = f.run(&done).unwrap();
        assert_eq!(again["changed"], false);
        assert_eq!(again["message"], finished["message"]);
        assert!(event.is_none());
        let (old_read, event) = f.run(&read_req).unwrap();
        assert_eq!(old_read["message"]["acks"][0], read["message"]["acks"][0]);
        assert_eq!(old_read["changed"], false);
        assert!(event.is_none());
        assert_eq!(
            f.run(&ack(id, Stage::Read, None, Some("changed")))
                .unwrap_err()
                .code,
            "ack_conflict"
        );
        assert_eq!(
            f.run(&ack(id, Stage::Done, Some(AckResult::Ok), None))
                .unwrap_err()
                .code,
            "ack_conflict"
        );
        let mut wrong = read_req;
        let Request::Ack { by, .. } = &mut wrong else {
            unreachable!()
        };
        *by = "crew-codex".into();
        assert_eq!(f.run(&wrong).unwrap_err().code, "invalid_argument");
    }

    #[test]
    fn list_cursor_survives_reopen_and_does_not_skip_matching_messages() {
        let f = Fixture::new();
        let (first, _) = f.run(&send("first")).unwrap();
        let mut other = send("other");
        let Request::Send { message, .. } = &mut other else {
            unreachable!()
        };
        message.to = "crew-codex".into();
        f.run(&other).unwrap();
        let (second, _) = f.run(&send("second")).unwrap();
        let before = fs::read(f.path()).unwrap();
        let (page, event) = f.run(&list(false, false, None, 1)).unwrap();
        assert!(event.is_none());
        assert_eq!(page["messages"][0]["id"], first["message"]["id"]);
        assert_eq!(page["has_more"], true);
        assert!(page["messages"][0].get("artifacts").is_none());
        let cursor = page["next_cursor"].as_str().unwrap().to_owned();
        // execute opens and loads the store afresh on every operation.
        let (page, _) = f.run(&list(false, false, Some(cursor), 1)).unwrap();
        assert_eq!(page["messages"][0]["id"], second["message"]["id"]);
        assert_eq!(page["has_more"], false);
        let cursor = page["next_cursor"].as_str().unwrap().to_owned();
        let (empty, _) = f.run(&list(false, false, Some(cursor.clone()), 1)).unwrap();
        assert_eq!(
            empty,
            json!({"messages":[],"next_cursor":cursor,"has_more":false})
        );
        assert_eq!(fs::read(f.path()).unwrap(), before);
        let other_store = Fixture::new();
        other_store.run(&send("new-store")).unwrap();
        assert_eq!(
            other_store
                .run(&list(false, false, Some(cursor), 1))
                .unwrap_err()
                .code,
            "invalid_argument"
        );
        assert_eq!(
            f.run(&list(false, false, Some("not-an-inbox-cursor".into()), 1))
                .unwrap_err()
                .code,
            "invalid_argument"
        );
    }

    #[test]
    fn open_includes_read_but_not_done_and_read_is_pure() {
        let f = Fixture::new();
        let (stored, _) = f.run(&send("open")).unwrap();
        let id = stored["message"]["id"].as_str().unwrap();
        let before = fs::read(f.path()).unwrap();
        let (read, event) = f.run(&Request::Read { id: id.into() }).unwrap();
        assert_eq!(read["message"]["acks"], json!([]));
        assert!(event.is_none());
        assert_eq!(fs::read(f.path()).unwrap(), before);
        f.run(&ack(id, Stage::Read, None, None)).unwrap();
        assert_eq!(
            f.run(&list(false, true, None, 20)).unwrap().0["messages"],
            json!([])
        );
        let open = f.run(&list(true, false, None, 20)).unwrap().0;
        assert_eq!(open["messages"].as_array().unwrap().len(), 1);
        assert_eq!(open["messages"][0]["read"], true);
        assert_eq!(open["messages"][0]["done"], Value::Null);
        f.run(&ack(id, Stage::Done, Some(AckResult::Ok), None))
            .unwrap();
        assert_eq!(
            f.run(&list(true, false, None, 20)).unwrap().0["messages"],
            json!([])
        );
        assert_eq!(
            f.run(&list(false, false, None, 20)).unwrap().0["messages"][0]["done"],
            "ok"
        );
    }

    #[test]
    fn known_offline_peers_and_reply_links_without_implicit_ack() {
        let f = Fixture::new();
        let (stored, _) = f.run(&send("initial")).unwrap();
        let id = stored["message"]["id"].as_str().unwrap();
        execute(&f.path(), &send("offline"), &[]).unwrap();
        let mut reply = send("reply");
        let Request::Send { message, .. } = &mut reply else {
            unreachable!()
        };
        message.kind = Kind::Answer;
        message.from = "crew-claude".into();
        message.to = "crew-codex".into();
        message.reply_to = Some(id.into());
        execute(&f.path(), &reply, &[]).unwrap();
        assert_eq!(
            f.run(&Request::Read { id: id.into() }).unwrap().0["message"]["acks"],
            json!([])
        );
        let Request::Send { message, .. } = &mut reply else {
            unreachable!()
        };
        message.key = "bad-reference".into();
        message.reply_to = Some("missing".into());
        assert_eq!(f.run(&reply).unwrap_err().code, "invalid_argument");
        let Request::Send { message, .. } = &mut reply else {
            unreachable!()
        };
        message.to = "never-known".into();
        assert_eq!(f.run(&reply).unwrap_err().code, "unknown_peer");
    }

    #[test]
    fn at_capacity_retry_read_ack_allowed_and_recipient_checked_before_capacity() {
        let f = Fixture::new();
        let mut store = Store::empty();
        let template = fixture_message("stored");
        for index in 0..MAX_MESSAGES {
            let mut message = template.clone();
            message.id = format!("msg_{}_{index}", store.identity);
            message.content.key = format!("key-{index}");
            message.content.artifacts.clear();
            store.messages.push(message);
        }
        save(&f.path(), &store).unwrap();
        let original = fs::read(f.path()).unwrap();
        let retry = Request::Send {
            caller: Some(4),
            message: store.messages[0].content.clone(),
        };
        assert_eq!(
            execute(&f.path(), &retry, &[]).unwrap().0["deduplicated"],
            true
        );
        assert_eq!(f.run(&send("new")).unwrap_err().code, "store_full");
        let mut unknown = send("new");
        let Request::Send { message, .. } = &mut unknown else {
            unreachable!()
        };
        message.to = "never-known".into();
        assert_eq!(f.run(&unknown).unwrap_err().code, "unknown_peer");
        assert_eq!(fs::read(f.path()).unwrap(), original);
        let id = &store.messages[0].id;
        f.run(&Request::Read { id: id.clone() }).unwrap();
        f.run(&ack(id, Stage::Read, None, None)).unwrap();
        assert_eq!(read(&f.path()).unwrap().messages.len(), MAX_MESSAGES);
    }

    #[test]
    fn concurrent_retry_stores_once() {
        let f = Fixture::new();
        let path = f.path();
        let req = send("concurrent");
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let req = req.clone();
                std::thread::spawn(move || execute(&path, &req, &["crew-claude".into()]).unwrap())
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(
            results.iter().filter(|(_, event)| event.is_some()).count(),
            1
        );
        assert_eq!(read(&f.path()).unwrap().messages.len(), 1);
        let id = &results[0].0["message"]["id"];
        assert!(
            results
                .iter()
                .all(|(value, _)| &value["message"]["id"] == id)
        );
    }

    #[test]
    fn missing_reads_and_corrupt_store_do_not_replace_data() {
        let f = Fixture::new();
        assert_eq!(
            f.run(&Request::Read {
                id: "missing".into()
            })
            .unwrap_err()
            .code,
            "message_not_found"
        );
        let (empty, _) = f.run(&list(false, false, None, 20)).unwrap();
        assert_eq!(
            empty,
            json!({"messages":[],"next_cursor":null,"has_more":false})
        );
        assert!(!f.path().exists());
        fs::write(f.path(), "broken JSON").unwrap();
        assert_eq!(f.run(&send("new")).unwrap_err().code, "internal");
        assert_eq!(fs::read(f.path()).unwrap(), b"broken JSON");
    }

    #[test]
    fn failed_atomic_replace_preserves_target_and_removes_staging_file() {
        let f = Fixture::new();
        fs::create_dir(f.path()).unwrap();
        let marker = f.path().join("preserve.txt");
        fs::write(&marker, "existing target").unwrap();
        assert_eq!(
            save(&f.path(), &Store::empty()).unwrap_err().code,
            "internal"
        );
        assert_eq!(fs::read(&marker).unwrap(), b"existing target");
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    }

    #[test]
    fn lock_timeout_is_explicit_and_release_allows_retry() {
        let f = Fixture::new();
        let open = || {
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(f.0.join("held.lock"))
                .unwrap()
        };
        let held = open();
        held.lock().unwrap();
        let contender = open();
        let started = Instant::now();
        let timeout = Duration::from_millis(40);
        let error = acquire_lock(&contender, timeout).unwrap_err();
        assert_eq!(error.code, "timeout");
        assert!(error.message.contains("40 ms"));
        assert!(started.elapsed() >= timeout);
        assert!(!f.path().exists());
        drop(held);
        acquire_lock(&contender, timeout).unwrap();
    }

    #[test]
    fn save_does_not_delete_other_stores_that_resemble_staging_files() {
        let f = Fixture::new();
        let prefix = format!(
            ".crew-inbox-write-{:x}-",
            Sha256::digest(f.path().file_name().unwrap().as_encoded_bytes())
        );
        let old = f.0.join(format!("{prefix}orphan"));
        let future = f.0.join(format!("{prefix}future"));
        let other = f.0.join(format!(
            ".crew-inbox-write-{:x}-other",
            Sha256::digest(b"other.json")
        ));
        let legacy = f.0.join(".crew-inbox-write-legacy");
        let directory = f.0.join(format!("{prefix}directory"));
        for path in [&other, &legacy] {
            let file = File::create(path).unwrap();
            file.set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)))
                .unwrap();
        }
        execute(&old, &send("other-store"), &["crew-claude".into()]).unwrap();
        File::create(&future)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(3600)),
            )
            .unwrap();
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("preserve.txt"), "not a staging file").unwrap();
        f.run(&send("cleanup")).unwrap();
        assert!(old.exists());
        assert_eq!(read(&old).unwrap().messages[0].content.key, "other-store");
        for path in [&future, &other, &legacy, &directory] {
            assert!(path.exists(), "{}", path.display());
        }
        assert_eq!(read(&f.path()).unwrap().messages.len(), 1);
    }
}
