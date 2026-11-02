//! Explicit, presentation-only bridge for a selected local Codex rollout.
use super::query;
use crate::api::Request;
use crate::metadata::{self, TextPatch};
use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCAN_LIMIT: usize = 1024 * 1024;
const LINE_LIMIT: usize = 256 * 1024;
const TTL_MS: u64 = 60_000;

#[derive(Debug, PartialEq)]
pub(super) struct Options {
    session_file: PathBuf,
    pane: usize,
    source: String,
    title: Option<String>,
    once: bool,
}

pub(super) fn parse(args: &[String]) -> Result<Option<Options>, String> {
    let mut values = BTreeMap::new();
    let mut once = false;
    let mut seen = BTreeSet::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--help" | "-h") {
            return Ok(None);
        }
        let (key, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
        if !seen.insert(key) {
            return Err(format!("duplicate option: {key}"));
        }
        if key == "--once" && inline.is_none() {
            once = true;
            continue;
        }
        if !matches!(key, "--session-file" | "--pane" | "--source" | "--title") {
            return Err(format!("unknown codex-progress option: {key}"));
        }
        let value = inline
            .or_else(|| args.next().map(String::as_str))
            .ok_or_else(|| format!("missing value for {key}"))?;
        values.insert(key, value);
    }
    let required = |key| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| format!("{key} is required"))
    };
    let pane = crate::api::parse_pane_id(required("--pane")?)
        .ok_or("--pane must be a pane ID such as p4")?;
    let source = required("--source")?;
    if !super::identifier(source) {
        return Err(
            "--source must use 1..80 ASCII letters, digits, colon, dot, underscore or hyphen"
                .into(),
        );
    }
    let title = values
        .get("--title")
        .map(|value| display_text(value, 80))
        .transpose()
        .map_err(|error| error.to_string())?;
    Ok(Some(Options {
        session_file: PathBuf::from(required("--session-file")?),
        pane,
        source: source.to_owned(),
        title,
        once,
    }))
}

fn display_text(value: &str, limit: usize) -> Result<String> {
    let text: String = value
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text: String = text.chars().take(limit).collect();
    ensure!(!text.is_empty(), "display text must not be empty");
    Ok(text)
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    epoch: String,
    agents: Vec<Pane>,
}

#[derive(Debug, Deserialize)]
struct Pane {
    pane_id: String,
    agent: Option<Reported>,
    detected_agent: Option<Detected>,
}

#[derive(Debug, Deserialize)]
struct Reported {
    agent: String,
    instance: u64,
    source: String,
}

#[derive(Debug, Deserialize)]
struct Detected {
    agent: String,
    process_id: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct Binding {
    epoch: String,
    detected_pid: Option<u32>,
    reported: Option<(u64, String)>,
}

fn binding(snapshot: Snapshot, pane: usize) -> Result<Binding> {
    let id = crate::api::pane_id_str(pane);
    let pane = snapshot
        .agents
        .into_iter()
        .find(|entry| entry.pane_id == id)
        .with_context(|| format!("{id} has no reported or detected Codex producer"))?;
    ensure!(
        pane.agent
            .as_ref()
            .is_none_or(|agent| agent.agent == "codex"),
        "{id} is explicitly reported as a different agent"
    );
    let detected_pid = pane
        .detected_agent
        .filter(|agent| agent.agent == "codex")
        .map(|agent| agent.process_id);
    let reported = pane.agent.map(|agent| (agent.instance, agent.source));
    ensure!(
        detected_pid.is_some() || reported.is_some(),
        "{id} has no reported or detected Codex producer"
    );
    Ok(Binding {
        epoch: snapshot.epoch,
        detected_pid,
        reported,
    })
}

fn current_binding(pane: usize) -> Result<Binding> {
    let value = query(Request::FederationSnapshot)
        .map_err(|error| anyhow!("{}: {}", error.code, error.message))?;
    binding(
        serde_json::from_value(value).context("invalid Crew identity snapshot")?,
        pane,
    )
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Record {
    #[serde(rename = "response_item")]
    ResponseItem {
        timestamp: Option<String>,
        payload: Item,
    },
    #[serde(rename = "event_msg")]
    Event { payload: Boundary },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Item {
    #[serde(rename = "message")]
    Message {
        role: String,
        phase: Option<String>,
        content: Vec<Content>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Content {
    #[serde(rename = "output_text")]
    Text { text: String },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Boundary {
    #[serde(rename = "task_started")]
    Started,
    #[serde(rename = "task_complete")]
    Complete,
    #[serde(rename = "turn_aborted")]
    Aborted,
    #[serde(other)]
    Other,
}

#[derive(Debug, PartialEq, Eq)]
enum Update {
    Commentary {
        text: String,
        timestamp: Option<String>,
    },
    Clear,
}

fn update(line: &[u8]) -> Option<Update> {
    match serde_json::from_slice::<Record>(line).ok()? {
        Record::ResponseItem {
            timestamp,
            payload:
                Item::Message {
                    role,
                    phase,
                    content,
                },
        } if role == "assistant" && phase.as_deref() == Some("commentary") => {
            let text = content
                .into_iter()
                .filter_map(|item| match item {
                    Content::Text { text } => Some(text),
                    Content::Other => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            Some(Update::Commentary {
                text: display_text(&text, 72).ok()?,
                timestamp,
            })
        }
        Record::ResponseItem {
            payload: Item::Message { role, phase, .. },
            ..
        } if role == "assistant" && phase.as_deref() == Some("final_answer") => Some(Update::Clear),
        Record::Event {
            payload: Boundary::Started | Boundary::Complete | Boundary::Aborted,
        } => Some(Update::Clear),
        _ => None,
    }
}

struct Tail {
    file: File,
    position: u64,
    pending: Vec<u8>,
    discard: bool,
}

impl Tail {
    fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path).context("cannot open the selected Codex session file")?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "--session-file must be a regular file");
        let position = metadata.len().saturating_sub(SCAN_LIMIT as u64);
        file.seek(SeekFrom::Start(position))?;
        Ok(Self {
            file,
            position,
            pending: Vec::new(),
            discard: position > 0,
        })
    }

    fn latest(&mut self) -> Result<Option<Update>> {
        ensure!(
            self.file.metadata()?.len() >= self.position,
            "the selected session file was truncated; restart with an explicitly selected session"
        );
        let mut latest = None;
        let mut read = 0;
        let mut chunk = [0u8; 8192];
        while read < SCAN_LIMIT {
            let limit = chunk.len().min(SCAN_LIMIT - read);
            let count = self.file.read(&mut chunk[..limit])?;
            if count == 0 {
                break;
            }
            read += count;
            self.position += count as u64;
            for byte in &chunk[..count] {
                if *byte == b'\n' {
                    if !self.discard
                        && let Some(next) = update(&self.pending)
                    {
                        latest = Some(next);
                    }
                    self.pending.clear();
                    self.discard = false;
                } else if !self.discard {
                    if self.pending.len() == LINE_LIMIT {
                        self.pending.clear();
                        self.discard = true;
                    } else {
                        self.pending.push(*byte);
                    }
                }
            }
        }
        Ok(latest)
    }
}

fn request(options: &Options, binding: &Binding, seq: u64, update: &Update) -> Request {
    let (title, state_labels, clear_presentation) = match update {
        Update::Commentary { text, .. } => (
            options
                .title
                .clone()
                .map_or_else(TextPatch::default, TextPatch::set),
            BTreeMap::from([("working".into(), Some(text.clone()))]),
            false,
        ),
        Update::Clear => (TextPatch::default(), BTreeMap::new(), true),
    };
    Request::Metadata(Box::new(metadata::Request {
        target: metadata::Target::Pane(options.pane),
        source: options.source.clone(),
        tokens: BTreeMap::new(),
        seq: Some(seq),
        ttl_ms: Some(TTL_MS),
        title,
        display_agent: TextPatch::default(),
        state_labels,
        agent: (!clear_presentation).then(|| "codex".into()),
        applies_to_source: if clear_presentation {
            None
        } else {
            binding.reported.as_ref().map(|(_, source)| source.clone())
        },
        clear_presentation,
    }))
}

pub(super) fn run(options: Options) -> Result<i32> {
    let mut tail = Tail::open(&options.session_file)?;
    let producer = current_binding(options.pane)?;
    let mut seq = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let mut last = None;
    let mut published = 0;
    loop {
        let current = current_binding(options.pane)?;
        ensure!(
            current == producer,
            "pane producer or Crew daemon changed; progress sync stopped"
        );
        if let Some(next) = tail.latest()?
            && last.as_ref() != Some(&next)
        {
            seq = seq.checked_add(1).context("progress sequence exhausted")?;
            let result = query(request(&options, &producer, seq, &next))
                .map_err(|error| anyhow!("{}: {}", error.code, error.message))?;
            ensure!(
                result["accepted"] == true,
                "progress metadata sequence was rejected; use a unique --source"
            );
            published += 1;
            last = Some(next);
        }
        if options.once {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    super::print_json(
        serde_json::json!({"pane_id":crate::api::pane_id_str(options.pane),
        "source":options.source,"updates":published,"once":options.once,
        "presentation_only":true,"ttl_ms":TTL_MS}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn commentary(text: &str) -> Vec<u8> {
        serde_json::to_vec(
            &json!({"type":"response_item","timestamp":"2026-11-02T02:11:13Z",
            "payload":{"type":"message","role":"assistant","phase":"commentary",
                "content":[{"type":"output_text","text":text}]}}),
        )
        .unwrap()
    }

    #[test]
    fn only_assistant_commentary_is_published_and_it_is_bounded() {
        let Some(Update::Commentary { text, .. }) =
            update(&commentary("Testing\n中文\u{1b}\u{202e}"))
        else {
            panic!()
        };
        assert_eq!(text, "Testing 中文");
        let Some(Update::Commentary { text, .. }) = update(&commentary(&"測".repeat(100))) else {
            panic!()
        };
        assert_eq!(text.chars().count(), 72);
        for payload in [
            json!({"type":"message","role":"user","phase":"commentary","content":[{"type":"input_text","text":"private user input"}]}),
            json!({"type":"message","role":"assistant","phase":"analysis","content":[{"type":"output_text","text":"private reasoning"}]}),
            json!({"type":"function_call","name":"shell","arguments":"secret command"}),
            json!({"type":"function_call_output","output":"secret tool output"}),
        ] {
            assert_eq!(
                update(
                    &serde_json::to_vec(&json!({"type":"response_item","payload":payload}))
                        .unwrap()
                ),
                None
            );
        }
        assert_eq!(update(br#"{"type":"event_msg","payload":{"type":"agent_message","phase":"commentary","message":"duplicate"}}"#), None);
        assert_eq!(
            update(br#"{"type":"event_msg","payload":{"type":"task_complete"}}"#),
            Some(Update::Clear)
        );
    }

    #[test]
    fn newline_partial_records_wait_and_oversized_records_do_not_block_later_progress() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!(
            "crew-codex-progress-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("session.jsonl");
        let mut writer = File::create(&path).unwrap();
        let line = commentary("running tests");
        writer.write_all(&line[..20]).unwrap();
        writer.flush().unwrap();
        let mut tail = Tail::open(&path).unwrap();
        assert_eq!(tail.latest().unwrap(), None);
        writer.write_all(&line[20..]).unwrap();
        writer.flush().unwrap();
        assert_eq!(tail.latest().unwrap(), None);
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        assert!(
            matches!(tail.latest().unwrap(), Some(Update::Commentary { text, .. }) if text == "running tests")
        );
        writer.write_all(&vec![b'x'; LINE_LIMIT + 1]).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.write_all(&commentary("next step")).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        assert!(
            matches!(tail.latest().unwrap(), Some(Update::Commentary { text, .. }) if text == "next step")
        );
        drop(tail);
        drop(writer);
        assert_eq!(
            root.canonicalize().unwrap().parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn binding_and_metadata_never_claim_lifecycle_completion() {
        let snapshot = |epoch: &str, pid: u32| {
            serde_json::from_value(json!({"epoch":epoch,"agents":[{
            "pane_id":"p4","agent":null,"detected_agent":{"agent":"codex","process_id":pid}}]}))
            .unwrap()
        };
        let first = binding(snapshot("one", 42), 4).unwrap();
        assert_ne!(first, binding(snapshot("two", 42), 4).unwrap());
        assert_ne!(first, binding(snapshot("one", 43), 4).unwrap());
        let options = Options {
            session_file: "selected.jsonl".into(),
            pane: 4,
            source: "test.progress".into(),
            title: Some("A session".into()),
            once: true,
        };
        let Request::Metadata(patch) = request(
            &options,
            &first,
            9,
            &Update::Commentary {
                text: "running tests".into(),
                timestamp: None,
            },
        ) else {
            panic!()
        };
        assert_eq!(patch.state_labels["working"], Some("running tests".into()));
        assert_eq!(patch.agent.as_deref(), Some("codex"));
        assert_eq!(patch.ttl_ms, Some(TTL_MS));
        assert!(patch.tokens.is_empty());
        let Request::Metadata(clear) = request(&options, &first, 10, &Update::Clear) else {
            panic!()
        };
        assert!(clear.clear_presentation);
        assert!(clear.agent.is_none());
        assert!(clear.state_labels.is_empty());
    }

    #[test]
    fn cli_requires_explicit_session_pane_and_source_and_keeps_value_text() {
        let args = |v: &[&str]| v.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        assert!(
            parse(&args(&[
                "--session-file",
                "selected.jsonl",
                "--source",
                "test"
            ]))
            .is_err()
        );
        let options = parse(&args(&[
            "--session-file",
            "selected.jsonl",
            "--pane",
            "p4",
            "--source",
            "test",
            "--title",
            "--help",
            "--once",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(options.title.as_deref(), Some("--help"));
        assert!(options.once);
    }
}
