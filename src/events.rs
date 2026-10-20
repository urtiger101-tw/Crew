//! Bounded server event journal. Waiting never holds the multiplexer lock.
use crate::api::{ApiError, ApiResult, Reply, Request as ApiRequest, lock};
use crate::server::Shared;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CAPACITY: usize = 512;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub cursor: u64,
    pub timestamp_ms: u64,
    pub kind: String,
    pub pane_id: Option<String>,
    pub data: Value,
}

pub struct Journal {
    epoch: String,
    cursor: u64,
    entries: VecDeque<Event>,
}

impl Default for Journal {
    fn default() -> Self {
        let epoch = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        Self {
            epoch,
            cursor: 0,
            entries: VecDeque::new(),
        }
    }
}

impl Journal {
    pub fn record(&mut self, kind: &str, pane: Option<usize>, data: Value) {
        self.cursor = self.cursor.saturating_add(1);
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(Event {
            cursor: self.cursor,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            kind: kind.to_string(),
            pane_id: pane.map(crate::api::pane_id_str),
            data,
        });
    }

    fn read(&self, after: u64, limit: usize, epoch: Option<&str>) -> Result<Value, ApiError> {
        if epoch.is_some_and(|e| e != self.epoch) {
            return Err(ApiError::new(
                "server_restarted",
                "event epoch changed; get a fresh snapshot and start at cursor 0",
            ));
        }
        if after > self.cursor {
            return Err(ApiError::new(
                "invalid_cursor",
                "cursor is ahead of this server; start at cursor 0",
            ));
        }
        let oldest = self
            .entries
            .front()
            .map_or(self.cursor.saturating_add(1), |e| e.cursor);
        let lost = after.saturating_add(1) < oldest;
        let entries: Vec<&Event> = self
            .entries
            .iter()
            .filter(|e| e.cursor > after)
            .take(limit)
            .collect();
        let next = entries.last().map_or(after, |e| e.cursor);
        Ok(
            json!({"epoch": self.epoch, "events": entries, "cursor": next, "latest_cursor": self.cursor, "events_lost": lost}),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Read {
        after: u64,
        limit: usize,
        epoch: Option<String>,
        timeout_ms: u64,
    },
}

pub fn parse(sub: &str, args: &[String]) -> Result<ApiRequest, String> {
    if !matches!(sub, "list" | "wait") {
        return Err(
            "usage: crew events list|wait [--after N] [--limit N] [--epoch E] [--timeout MS]"
                .into(),
        );
    }
    let mut after = 0;
    let mut limit = 100;
    let mut timeout_ms = if sub == "wait" { 30_000 } else { 0 };
    let mut epoch = None;
    let mut seen = std::collections::HashSet::new();
    let mut i = 0;
    while i < args.len() {
        let (key, inline) = args[i]
            .split_once('=')
            .map_or((args[i].as_str(), None), |(k, v)| (k, Some(v)));
        if !seen.insert(key) {
            return Err(format!("duplicate option: {key}"));
        }
        if !matches!(key, "--after" | "--limit" | "--epoch" | "--timeout") {
            return Err(format!("unknown option: {key}"));
        }
        i += 1;
        let value = match inline {
            Some(v) => v,
            None => {
                let v = args
                    .get(i)
                    .ok_or_else(|| format!("missing value for {key}"))?;
                i += 1;
                v
            }
        };
        let number = || {
            value
                .parse::<u64>()
                .map_err(|_| format!("{key} must be a nonnegative integer"))
        };
        match key {
            "--after" => after = number()?,
            "--limit" => {
                let n = number()?;
                if !(1..=512).contains(&n) {
                    return Err("--limit must be 1..512".into());
                }
                limit = n as usize;
            }
            "--epoch" => epoch = Some(value.to_string()),
            "--timeout" if sub == "wait" => timeout_ms = number()?,
            _ => return Err("--timeout requires events wait".into()),
        }
    }
    if timeout_ms > 600_000 {
        return Err("--timeout must not exceed 600000 ms".into());
    }
    Ok(ApiRequest::Events(Request::Read {
        after,
        limit,
        epoch,
        timeout_ms,
    }))
}

pub fn handle(shared: &Shared, req: Request) -> ApiResult {
    let Request::Read {
        after,
        limit,
        epoch,
        timeout_ms,
    } = req;
    if !(1..=CAPACITY).contains(&limit) || timeout_ms > 600_000 {
        return Err(ApiError::invalid(
            "limit must be 1..512 and timeout <= 600000 ms",
        ));
    }
    let end = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let result = { lock(shared).events.read(after, limit, epoch.as_deref())? };
        if result["events"].as_array().is_some_and(|v| !v.is_empty()) || Instant::now() >= end {
            return Ok(Reply::Json(result));
        }
        std::thread::sleep(
            Duration::from_millis(25).min(end.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_history_reports_loss_and_supports_pagination() {
        let mut j = Journal::default();
        for i in 0..600 {
            j.record("pane.output", Some(1), json!({"bytes":i}));
        }
        let r = j.read(0, 2, None).unwrap();
        assert_eq!(r["events_lost"], true);
        assert_eq!(r["events"].as_array().unwrap().len(), 2);
        assert_eq!(r["cursor"], 90);
        assert_eq!(j.read(90, 1, None).unwrap()["events"][0]["cursor"], 91);
        assert_eq!(j.entries.len(), CAPACITY);
    }
    #[test]
    fn restart_and_future_cursor_are_explicit_errors() {
        let j = Journal::default();
        assert_eq!(
            j.read(0, 1, Some("old")).unwrap_err().code,
            "server_restarted"
        );
        assert_eq!(j.read(1, 1, None).unwrap_err().code, "invalid_cursor");
    }
    #[test]
    fn parser_rejects_bad_limits_and_unknown_options() {
        let args = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert!(parse("wait", &args("--after=1 --timeout 5")).is_ok());
        for bad in [
            "--limit 0",
            "--limit 513",
            "--timeout 600001",
            "--after 0 --after 1",
            "--bad x",
        ] {
            assert!(parse("wait", &args(bad)).is_err());
        }
    }
}
