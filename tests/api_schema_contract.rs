//! `crew api schema` must publish every error code the source can emit.
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn schema() -> Value {
    let root = std::env::temp_dir().join(format!("crew-schema-{}-中文 space", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("CREW_") || upper == "CREW" {
            command.env_remove(key);
        }
    }
    let out = command
        .args(["api", "schema"])
        .current_dir(&root)
        .env(
            "CREW_SOCKET",
            format!("crew-schema-{}.sock", std::process::id()),
        )
        .env("CREW_CONFIG", root.join("unused-config.toml"))
        .env("CREW_STATE", root.join("unused-state.json"))
        .env("CREW_PLUGIN_HOME", root.join("unused-plugins"))
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn codes(schema: &Value) -> Vec<(String, String)> {
    schema["error_codes"]
        .as_array()
        .expect("error_codes array")
        .iter()
        .map(|e| {
            (
                e["code"].as_str().unwrap().to_string(),
                e["meaning"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn schema_lists_every_emitted_error_code() {
    let schema = schema();
    assert!(schema["methods"].as_array().is_some_and(|m| !m.is_empty()));
    let listed: Vec<String> = codes(&schema).into_iter().map(|(c, _)| c).collect();
    let re = Regex::new(r#"ApiError::new\(\s*"([a-z_]+)""#).unwrap();
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut missing = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for cap in re.captures_iter(&text) {
            if !listed.contains(&cap[1].to_string()) {
                missing.push(format!("{} in {}", &cap[1], file.display()));
            }
        }
    }
    assert!(missing.is_empty(), "unregistered error codes: {missing:?}");
}

#[test]
fn error_codes_are_sorted_unique_and_described() {
    let codes = codes(&schema());
    for pair in codes.windows(2) {
        assert!(
            pair[0].0 < pair[1].0,
            "not sorted/unique: {} {}",
            pair[0].0,
            pair[1].0
        );
    }
    assert!(codes.iter().all(|(_, m)| !m.trim().is_empty()));
    for required in [
        "invalid_argument",
        "pane_not_found",
        "server_not_running",
        "ipc_permission_denied",
        "method_not_found",
    ] {
        assert!(
            codes.iter().any(|(c, _)| c == required),
            "missing {required}"
        );
    }
}
