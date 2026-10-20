use super::manifest::Manifest;
use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub const BUNDLED: &[(&str, &str)] = &[
    (
        "crew.workspaces",
        include_str!("../../plugins/crew.workspaces/crew-plugin.toml"),
    ),
    (
        "crew.worktrees",
        include_str!("../../plugins/crew.worktrees/crew-plugin.toml"),
    ),
];

pub fn bundled(source: &str) -> Option<&'static str> {
    let id = source.strip_prefix("builtin:").unwrap_or(source);
    BUNDLED
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, raw)| *raw)
}

pub fn search(query: &str) -> Result<Vec<Value>> {
    let query = query.to_lowercase();
    BUNDLED
        .iter()
        .filter_map(|(id, raw)| {
            let parsed = Manifest::parse(raw);
            match parsed {
                Ok(m)
                    if format!("{} {} {}", m.id, m.name, m.description)
                        .to_lowercase()
                        .contains(&query) =>
                {
                    Some(Ok(json!({"id": id, "name": m.name, "version": m.version,
                    "description": m.description, "source": format!("builtin:{id}"),
                    "kind": "bundled", "platforms": m.platforms, "actions": m.actions,
                    "included_in_binary": true})))
                }
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            }
        })
        .collect()
}

/// Only a GitHub repository shorthand is accepted, never a URL or git option.
pub fn github_source(source: &str) -> Result<(String, String)> {
    let parts: Vec<_> = source.split('/').collect();
    ensure!(
        parts.len() >= 2 && parts.len() <= 12,
        "source must be owner/repo[/subdirectory]"
    );
    for part in &parts {
        ensure!(
            !part.is_empty()
                && !matches!(*part, "." | "..")
                && !part.starts_with('-')
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')),
            "invalid GitHub source component"
        );
    }
    Ok((format!("{}/{}", parts[0], parts[1]), parts[2..].join("/")))
}

pub fn git_ref(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 200
            && !value.starts_with('-')
            && !value.contains("..")
            && !value.contains("@{")
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'.' | b'_' | b'-')),
        "ref must be a branch, tag or commit without git revision operators"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repository_sources_cannot_escape_or_inject_transport_arguments() {
        assert_eq!(
            github_source("owner/repo/plugin").unwrap(),
            ("owner/repo".into(), "plugin".into())
        );
        for bad in [
            "https://github.com/a/b",
            "a/b/../c",
            "-c/b",
            "a/b/",
            "a/b;x",
            "a/b\\x",
        ] {
            assert!(github_source(bad).is_err(), "{bad}");
        }
        for bad in [
            "--upload-pack=evil",
            "a..b",
            "HEAD^{tree}",
            "a b",
            "a;touch",
        ] {
            assert!(git_ref(bad).is_err(), "{bad}");
        }
        assert!(git_ref("v1.2.3").is_ok());
    }
}
