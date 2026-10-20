use super::{Request, Target};
use crate::api::{Request as ApiRequest, parse_pane_id};
use std::collections::{BTreeMap, BTreeSet};

pub fn parse(group: &str, args: &[String]) -> Result<ApiRequest, String> {
    let mut positional = Vec::new();
    let mut opts = BTreeMap::new();
    let mut tokens = BTreeMap::new();
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
            "--source" | "--seq" | "--ttl-ms" | "--token" | "--clear-token"
        ) {
            return Err(format!("unknown option: {flag}"));
        }
        if !matches!(flag, "--token" | "--clear-token") && !seen.insert(flag) {
            return Err(format!("duplicate option: {flag}"));
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
        if matches!(flag, "--token" | "--clear-token") {
            let (key, value) = if flag == "--token" {
                let (key, value) = value.split_once('=').ok_or("--token requires KEY=VALUE")?;
                (key.to_string(), Some(value.to_string()))
            } else {
                (value, None)
            };
            if tokens.contains_key(&key) {
                return Err(format!("duplicate token key: {key}"));
            }
            tokens.insert(key, value);
        } else {
            opts.insert(flag, value);
        }
    }
    if positional.len() != 1 {
        return Err(format!(
            "usage: crew {group} report-metadata TARGET --source ID --token KEY=VALUE [OPTIONS]"
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
    };
    req.validate().map_err(|e| e.message)?;
    Ok(ApiRequest::Metadata(req))
}
