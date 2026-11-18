//! Documentation contract: the agent quick reference covers every bridge method and its links
//! resolve, and the README points to it from a quick start placed before the release notes.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Methods from `crew api schema`. The schema is printed as a bare JSON object.
fn schema_methods() -> Vec<String> {
    let root = std::env::temp_dir().join(format!("crew-docs-contract-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("CREW_") || upper == "CREW" {
            command.env_remove(key);
        }
    }
    let output = command
        .args(["api", "schema"])
        .current_dir(&root)
        .env(
            "CREW_SOCKET",
            format!("crew-docs-{}.sock", std::process::id()),
        )
        .env("CREW_CONFIG", root.join("unused-config.toml"))
        .env("CREW_STATE", root.join("unused-state.json"))
        .env("CREW_PLUGIN_HOME", root.join("unused-plugins"))
        .env("PATH", "")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        output.status.success(),
        "api schema failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let schema: Value = serde_json::from_slice(&output.stdout).unwrap();
    schema["methods"]
        .as_array()
        .expect("schema has a methods array")
        .iter()
        .map(|m| m.as_str().expect("method is a string").to_string())
        .collect()
}

fn link_targets(doc: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = doc;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(')') else { break };
        targets.push(after[..end].trim().to_string());
        rest = &after[end + 1..];
    }
    targets
}

#[test]
fn quick_reference_covers_every_schema_method() {
    let doc = read(&repo_root().join("docs/agent-quick-reference.md"));
    let methods = schema_methods();
    assert!(!methods.is_empty(), "schema lists no methods");
    let missing: Vec<&String> = methods
        .iter()
        .filter(|method| {
            let (group, sub) = method
                .split_once('.')
                .unwrap_or_else(|| panic!("method without a group: {method}"));
            !doc.contains(&format!("{group} {sub}"))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "agent quick reference lacks these methods: {missing:?}"
    );
}

#[test]
fn readme_has_quick_start_before_release_notes() {
    let readme = read(&repo_root().join("README.md"));
    let quick_start = readme
        .find("Quick start")
        .expect("README has no Quick start section");
    let release_notes = readme
        .find("## Crew 0.6.0")
        .expect("README has no Crew 0.6.0 heading");
    assert!(
        quick_start < release_notes,
        "Quick start must come before the release notes"
    );
    assert!(readme.contains("docs/agent-quick-reference.md"));
}

#[test]
fn quick_reference_links_resolve() {
    let path = repo_root().join("docs/agent-quick-reference.md");
    let doc = read(&path);
    let base = path.parent().unwrap();
    let mut checked = 0;
    for target in link_targets(&doc) {
        if target.contains("://") || target.starts_with("mailto:") || target.starts_with('#') {
            continue;
        }
        let file = target.split('#').next().unwrap_or_default();
        assert!(base.join(file).exists(), "broken link target: {target}");
        checked += 1;
    }
    assert!(
        checked >= 3,
        "expected relative links in the quick reference"
    );
}
