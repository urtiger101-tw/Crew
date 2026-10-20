use super::{Request, Target, TextPatch};
use crate::api::{Request as ApiRequest, parse_pane_id};
use std::collections::{BTreeMap, BTreeSet};

pub fn parse(group: &str, args: &[String]) -> Result<ApiRequest, String> {
    let mut positional = Vec::new();
    let mut opts = BTreeMap::new();
    let mut tokens = BTreeMap::new();
    let mut state_labels = BTreeMap::new();
    let mut title = TextPatch::default();
    let mut display_agent = TextPatch::default();
    let mut clear_presentation = false;
    let mut seen = BTreeSet::new();
    let mut i = 0;
    while i < args.len() {
        let item = &args[i];
        i += 1;
        if item == "--" {
            positional.extend_from_slice(&args[i..]);
            break;
        }
        if !item.starts_with('-') {
            positional.push(item.clone());
            continue;
        }
        let (flag, inline) = item
            .split_once('=')
            .map_or((item.as_str(), None), |(k, v)| (k, Some(v)));
        if !matches!(
            flag,
            "--source"
                | "--seq"
                | "--ttl-ms"
                | "--token"
                | "--clear-token"
                | "--title"
                | "--clear-title"
                | "--display-agent"
                | "--clear-display-agent"
                | "--state-label"
                | "--clear-state-label"
                | "--agent"
                | "--applies-to-source"
                | "--clear-presentation"
        ) {
            return Err(format!("unknown option: {flag}"));
        }
        if !matches!(
            flag,
            "--token" | "--clear-token" | "--state-label" | "--clear-state-label"
        ) && !seen.insert(flag)
        {
            return Err(format!("duplicate option: {flag}"));
        }
        if matches!(
            flag,
            "--clear-title" | "--clear-display-agent" | "--clear-presentation"
        ) {
            if inline.is_some() {
                return Err(format!("{flag} does not accept a value"));
            }
            match flag {
                "--clear-title" if title.missing() => title = TextPatch::clear(),
                "--clear-display-agent" if display_agent.missing() => {
                    display_agent = TextPatch::clear()
                }
                "--clear-presentation" => clear_presentation = true,
                _ => return Err(format!("duplicate presentation field: {flag}")),
            }
            continue;
        }
        let value = match inline {
            Some(v) => v.to_string(),
            None => {
                let value = args
                    .get(i)
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                i += 1;
                value.clone()
            }
        };
        if matches!(
            flag,
            "--token" | "--clear-token" | "--state-label" | "--clear-state-label"
        ) {
            let (key, value) = if matches!(flag, "--token" | "--state-label") {
                let (key, value) = value.split_once('=').ok_or("option requires KEY=VALUE")?;
                (key.to_string(), Some(value.to_string()))
            } else {
                (value, None)
            };
            let values = if matches!(flag, "--state-label" | "--clear-state-label") {
                &mut state_labels
            } else {
                &mut tokens
            };
            if values.contains_key(&key) {
                return Err(format!("duplicate metadata key: {key}"));
            }
            values.insert(key, value);
        } else if flag == "--title" {
            if !title.missing() {
                return Err("duplicate title".into());
            }
            title = TextPatch::set(value);
        } else if flag == "--display-agent" {
            if !display_agent.missing() {
                return Err("duplicate display-agent".into());
            }
            display_agent = TextPatch::set(value);
        } else {
            opts.insert(flag, value);
        }
    }
    if positional.len() != 1 {
        return Err(format!(
            "usage: crew {group} report-metadata TARGET --source ID [--token KEY=VALUE|--title TEXT|PRESENTATION_OPTIONS]"
        ));
    }
    let target = positional.pop().unwrap();
    let target = match group {
        "pane" => Target::Pane(parse_pane_id(&target).ok_or("invalid pane id; expected pN or %N")?),
        "workspace" => Target::Workspace(target),
        _ => return Err("metadata supports pane or workspace targets".into()),
    };
    let number = |flag| -> Result<Option<u64>, String> {
        opts.get(flag)
            .map(|v| {
                if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(format!("{flag} must be a nonnegative integer"));
                }
                v.parse::<u64>()
                    .map_err(|_| format!("{flag} must be a nonnegative integer"))
            })
            .transpose()
    };
    let req = Request {
        target,
        source: opts
            .get("--source")
            .ok_or("missing required --source")?
            .clone(),
        tokens,
        seq: number("--seq")?,
        ttl_ms: number("--ttl-ms")?,
        title,
        display_agent,
        state_labels,
        agent: opts.get("--agent").cloned(),
        applies_to_source: opts.get("--applies-to-source").cloned(),
        clear_presentation,
    };
    req.validate().map_err(|e| e.message)?;
    Ok(ApiRequest::Metadata(Box::new(req)))
}
