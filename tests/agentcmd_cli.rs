use std::process::Command;

#[test]
fn agentcmd_cli_reports_missing_cargo_and_rejects_offline_without_source() {
    let root = std::env::temp_dir().join(format!("crew-agentcmd-cli-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_crew"))
            .args(args)
            .env("PATH", "")
            .env("CREW_PLUGIN_HOME", root.join("plugins"))
            .env("CREW_STATE", root.join("state"))
            .env("CREW_SOCKET", root.join("socket"))
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
        root.to_str().unwrap(),
        "--yes",
    ]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.to_ascii_lowercase().contains("cargo"), "{stderr}");
    std::fs::remove_dir_all(root).unwrap();
}
