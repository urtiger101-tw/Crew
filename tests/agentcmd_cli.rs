use std::process::Command;

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        if let (Ok(path), Ok(temp)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize())
            && path.parent() == Some(temp.as_path())
        {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[test]
fn agentcmd_cli_reports_missing_cargo_and_rejects_offline_without_source() {
    let path = std::env::temp_dir().join(format!(
        "crew-agentcmd-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    let root = TempDir(path);
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_crew"))
            .args(args)
            .env("PATH", "")
            .env("CREW_PLUGIN_HOME", root.0.join("plugins"))
            .env("CREW_STATE", root.0.join("state"))
            .env("CREW_SOCKET", root.0.join("socket"))
            .output()
            .unwrap()
    };
    let offline = run(&["plugin", "agentcmd", "install", "--offline", "--yes"]);
    assert!(!offline.status.success());
    assert!(String::from_utf8_lossy(&offline.stderr).contains("--source"));
    let missing = run(&[
        "plugin",
        "agentcmd",
        "install",
        "--source",
        root.0.to_str().unwrap(),
        "--yes",
    ]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.to_ascii_lowercase().contains("cargo"), "{stderr}");
}
