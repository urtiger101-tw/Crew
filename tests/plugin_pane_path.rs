use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Server {
    root: PathBuf,
    socket: String,
    config: PathBuf,
    state: PathBuf,
    plugin_home: PathBuf,
}

impl Server {
    fn new() -> Self {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("crew-plugin-pane-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        let config = root.join("config.toml");
        let shell = if cfg!(windows) {
            PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
                .to_string_lossy()
                .into_owned()
        } else {
            "/bin/sh".into()
        };
        let shell = if cfg!(windows) {
            format!("'{shell}' -NoLogo -NoProfile")
        } else {
            format!("'{shell}'")
        };
        fs::write(
            &config,
            format!("persist = false\nshell = {}\n", json!(shell)),
        )
        .unwrap();
        Self {
            socket: format!("crew-plugin-pane-{suffix}.sock"),
            state: root.join("state.json"),
            plugin_home: root.join("home"),
            config,
            root,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        command
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_STATE", &self.state)
            .env("CREW_PLUGIN_HOME", &self.plugin_home)
            .env("CREW_CONFIG", &self.config)
            .env("CREW_LOG", self.root.join("server.log"))
            .env_remove("CREW_PANE_ID");
        command
    }

    fn run(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{args:?}: {error}: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    fn plugin(&self, id: &str, command: &str) -> (PathBuf, PathBuf) {
        let root = self.root.join(id);
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let manifest = root.join("crew-plugin.toml");
        fs::write(
            &manifest,
            format!(
                "id='{id}'\nname='Fixture'\nversion='1.0.0'\nmin_crew_version='{}'\nplatforms=['windows','linux','macos']\n[[path_dirs]]\npath='bin'\n",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
        let executable = bin.join(format!(
            "{command}{}",
            if cfg!(windows) { ".cmd" } else { "" }
        ));
        fs::write(
            &executable,
            if cfg!(windows) {
                "@echo off\r\necho FIXTURE_WORKS\r\n"
            } else {
                "#!/bin/sh\nprintf 'FIXTURE_WORKS\\n'\n"
            },
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        self.run(&[
            "plugin",
            "link",
            root.to_str().unwrap(),
            "--yes",
            "--disabled",
        ]);
        // Canonicalize to match the directory handed to child processes without Windows'
        // verbatim prefix, which the plugin contract deliberately removes.
        let canonical = fs::canonicalize(&bin).unwrap();
        #[cfg(windows)]
        let canonical = PathBuf::from(canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
        (manifest, canonical)
    }

    fn pane(&self, id: &str) -> Value {
        self.run(&["pane", "get", id])["result"]["pane"].clone()
    }

    fn newest_pane(&self) -> String {
        self.run(&["pane", "list"])["result"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pane| pane["pane_id"].as_str().unwrap())
            .max_by_key(|id| id[1..].parse::<usize>().unwrap())
            .unwrap()
            .to_owned()
    }

    fn capture(&self, pane: &str, expression: &str, tag: &str) -> String {
        let output = self.root.join(format!("{tag}.txt"));
        let done = self.root.join(format!("{tag}.done"));
        let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
        let line = if cfg!(windows) {
            format!(
                "[IO.File]::WriteAllText('{}', [string]({expression})); [IO.File]::WriteAllText('{}', 'done')",
                quote(&output),
                quote(&done)
            )
        } else {
            format!(
                "{expression} > '{}'; printf done > '{}'",
                quote(&output),
                quote(&done)
            )
        };
        self.run(&["pane", "run", pane, &line]);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done.exists() {
            assert!(
                Instant::now() < deadline,
                "pane {pane} did not write {tag}; pane output: {}; log: {}",
                self.run(&["pane", "read", pane]),
                fs::read_to_string(self.root.join("server.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        fs::read_to_string(output).unwrap()
    }

    fn path(&self, pane: &str, tag: &str) -> Vec<PathBuf> {
        let expr = if cfg!(windows) {
            "$env:Path"
        } else {
            "printf '%s' \"$PATH\""
        };
        let value = self.capture(pane, expr, tag);
        std::env::split_paths(value.trim_start_matches('\u{feff}')).collect()
    }

    fn has_command(&self, pane: &str, name: &str, tag: &str) -> bool {
        let expression = if cfg!(windows) {
            format!(
                "$(if (Get-Command {name} -CommandType Application -ErrorAction SilentlyContinue) {{ 'yes' }} else {{ 'no' }})"
            )
        } else {
            format!("if command -v {name} >/dev/null 2>&1; then printf yes; else printf no; fi")
        };
        self.capture(pane, &expression, tag).trim() == "yes"
    }

    fn attach(&self, workspace: &str) -> impl Write {
        use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Stream, prelude::*};
        let name = if GenericNamespaced::is_supported() {
            self.socket
                .clone()
                .to_ns_name::<GenericNamespaced>()
                .unwrap()
        } else {
            std::env::temp_dir()
                .join(&self.socket)
                .to_fs_name::<GenericFilePath>()
                .unwrap()
        };
        let stream = Stream::connect(name).unwrap();
        let (mut reader, mut writer) = stream.split();
        std::thread::spawn(move || {
            let mut size = [0u8; 4];
            while reader.read_exact(&mut size).is_ok() {
                let mut frame = vec![0; u32::from_le_bytes(size) as usize];
                if reader.read_exact(&mut frame).is_err() {
                    break;
                }
            }
        });
        send_frame(
            &mut writer,
            json!({"Hello": {"Attach": {
                "session": workspace, "create": false, "cols": 120, "rows": 30,
                "cwd": null, "detached": false,
                "env": {"no_color": false, "dumb": false, "ascii": true, "lang": null}
            }}}),
        );
        writer
    }
}

fn send_frame(writer: &mut impl Write, message: Value) {
    let bytes = serde_json::to_vec(&message).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .unwrap();
    writer.write_all(&bytes).unwrap();
    writer.flush().unwrap();
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn confirm_before_bindings_prepare_paths_when_confirmation_is_disabled() {
    let server = Server::new();
    let config = fs::read_to_string(&server.config).unwrap();
    fs::write(
        &server.config,
        format!(
            "{config}confirm_close = false\n[bind]\nn = 'confirm-before new-window'\ns = 'confirm-before split-window -h'\nd = 'confirm-before \"confirm-before new-window\"'\n"
        ),
    )
    .unwrap();
    let (_, bin) = server.plugin("a.alpha", "crewfixturealpha");
    server.run(&["plugin", "enable", "a.alpha"]);
    server.run(&["workspace", "create", "confirm-binding"]);
    let mut client = server.attach("confirm-binding");
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    for (key, tag) in [
        ('n', "confirm-new"),
        ('s', "confirm-split"),
        ('d', "confirm-nested"),
    ] {
        let before = server.newest_pane();
        for key in [
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
        ] {
            send_frame(&mut client, json!({"Event": Event::Key(key)}));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let created = loop {
            let latest = server.newest_pane();
            if latest != before {
                break latest;
            }
            assert!(Instant::now() < deadline, "{tag} did not create a pane");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(server.pane(&created)["plugin_path_dirs"], json!([bin]));
        assert_eq!(server.path(&created, tag).first(), Some(&bin));
        assert!(server.has_command(&created, "crewfixturealpha", &format!("{tag}-command")));
    }
}

#[test]
fn fresh_panes_get_enabled_paths_and_record_their_own_snapshots() {
    let server = Server::new();
    let (manifest_a, bin_a) = server.plugin("a.alpha", "crewfixturealpha");
    let (_, bin_z) = server.plugin("z.omega", "crewfixtureomega");
    let base_a = server.root.join("base one");
    let base_b = server.root.join("base-two");
    fs::create_dir_all(&base_a).unwrap();
    fs::create_dir_all(&base_b).unwrap();
    let base = vec![base_a, base_b];
    let path = std::env::join_paths(&base).unwrap().into_string().unwrap();
    let key = if cfg!(windows) { "Path" } else { "PATH" };
    server.run(&[
        "workspace",
        "create",
        "explicit",
        "--env",
        &format!("{key}={path}"),
    ]);
    let original = server.newest_pane();
    assert_eq!(server.pane(&original)["plugin_path_dirs"], json!([]));
    assert_eq!(server.path(&original, "original-before"), base);

    server.run(&["workspace", "create", "inherited"]);
    let inherited = server.newest_pane();
    let inherited_base = server.path(&inherited, "inherited-before");

    server.run(&["plugin", "enable", "z.omega"]);
    server.run(&["pane", "split", &original, "--direction", "right"]);
    let split_z = server.newest_pane();
    assert_eq!(server.pane(&split_z)["plugin_path_dirs"], json!([bin_z]));
    assert_eq!(
        server.path(&split_z, "split-z"),
        [vec![bin_z.clone()], base.clone()].concat()
    );
    assert!(server.has_command(&split_z, "crewfixtureomega", "split-z-command"));

    server.run(&["pane", "split", &inherited, "--direction", "right"]);
    let inherited_z = server.newest_pane();
    assert_eq!(
        server.path(&inherited_z, "inherited-z"),
        [vec![bin_z.clone()], inherited_base].concat()
    );

    server.run(&["plugin", "enable", "a.alpha"]);
    server.run(&["tab", "create", "both", "--workspace", "explicit"]);
    let both = server.newest_pane();
    assert_eq!(
        server.pane(&both)["plugin_path_dirs"],
        json!([bin_a, bin_z])
    );
    assert_eq!(
        server.path(&both, "both"),
        [vec![bin_a.clone(), bin_z.clone()], base.clone()].concat()
    );

    // Exercise the attached-client prefix binding, which starts a window on its event thread.
    let mut client = server.attach("explicit");
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    for key in [
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
    ] {
        send_frame(&mut client, json!({"Event": Event::Key(key)}));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let interactive = loop {
        let latest = server.newest_pane();
        if latest != both {
            break latest;
        }
        assert!(Instant::now() < deadline, "prefix did not create a window");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        server.pane(&interactive)["plugin_path_dirs"],
        json!([bin_a, bin_z])
    );
    assert_eq!(
        server.path(&interactive, "interactive"),
        [vec![bin_a.clone(), bin_z.clone()], base.clone()].concat()
    );
    for args in [
        &["new-window", "-n", "legacy"][..],
        &["split-window", "-h"][..],
        &["new", "-d", "-s", "legacy-session"][..],
    ] {
        let output = server.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let created = server.newest_pane();
        assert_eq!(
            server.pane(&created)["plugin_path_dirs"],
            json!([bin_a, bin_z]),
            "{args:?}"
        );
    }

    server.run(&["plugin", "disable", "z.omega"]);
    let before_disabled_ui = server.newest_pane();
    for key in [
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
    ] {
        send_frame(&mut client, json!({"Event": Event::Key(key)}));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let disabled_ui = loop {
        let latest = server.newest_pane();
        if latest != before_disabled_ui {
            break latest;
        }
        assert!(
            Instant::now() < deadline,
            "prefix did not create a window after disable"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        server.pane(&disabled_ui)["plugin_path_dirs"],
        json!([bin_a])
    );
    assert_eq!(
        server.pane(&interactive)["plugin_path_dirs"],
        json!([bin_a, bin_z])
    );
    drop(client);
    server.run(&["pane", "split", &split_z, "--direction", "down"]);
    let split_a = server.newest_pane();
    assert_eq!(server.pane(&split_a)["plugin_path_dirs"], json!([bin_a]));
    assert_eq!(
        server.path(&split_a, "split-a"),
        [vec![bin_a.clone()], base.clone()].concat()
    );
    assert!(!server.has_command(&split_a, "crewfixtureomega", "disabled-z-command"));
    assert_eq!(server.pane(&split_z)["plugin_path_dirs"], json!([bin_z]));
    assert_eq!(
        server.path(&split_z, "old-split-after-disable"),
        [vec![bin_z.clone()], base.clone()].concat()
    );
    assert!(server.has_command(&split_z, "crewfixtureomega", "old-z-command"));
    assert_eq!(server.path(&original, "original-after"), base);

    fs::write(
        &manifest_a,
        format!("{}\n# changed", fs::read_to_string(&manifest_a).unwrap()),
    )
    .unwrap();
    server.run(&["pane", "split", &original, "--direction", "down"]);
    let stale = server.newest_pane();
    let stale_pane = server.pane(&stale);
    assert_eq!(stale_pane["plugin_path_dirs"], json!([]));
    assert!(
        stale_pane["plugin_path_diagnostics"]
            .to_string()
            .contains("manifest changed")
    );
    assert_eq!(server.path(&stale, "stale"), base);

    fs::write(server.plugin_home.join("registry.json"), b"{invalid").unwrap();
    server.run(&["tab", "create", "corrupt", "--workspace", "explicit"]);
    let corrupt = server.newest_pane();
    let corrupt_pane = server.pane(&corrupt);
    assert_eq!(corrupt_pane["plugin_path_dirs"], json!([]));
    assert!(
        corrupt_pane["plugin_path_diagnostics"]
            .to_string()
            .contains("invalid plugin registry")
    );
    assert_eq!(server.path(&corrupt, "corrupt"), base);

    let panes = server.run(&["pane", "list"]);
    let panes = panes["result"]["panes"].as_array().unwrap();
    for id in [&original, &split_z, &both, &split_a, &stale, &corrupt] {
        let row = panes
            .iter()
            .find(|pane| pane["pane_id"] == id.as_str())
            .unwrap();
        assert_eq!(row["plugin_path_dirs"], server.pane(id)["plugin_path_dirs"]);
    }
}

#[test]
fn restored_panes_resolve_the_current_plugins_and_keep_saved_path_overrides() {
    let server = Server::new();
    let config = fs::read_to_string(&server.config).unwrap();
    fs::write(
        &server.config,
        config.replace("persist = false", "persist = true"),
    )
    .unwrap();
    let (_, bin_a) = server.plugin("a.alpha", "crewfixturealpha");
    let (_, bin_z) = server.plugin("z.omega", "crewfixtureomega");
    let base = server.root.join("saved-base");
    fs::create_dir_all(&base).unwrap();
    server.run(&["plugin", "enable", "a.alpha"]);
    server.run(&[
        "workspace",
        "create",
        "restore-me",
        "--env",
        &format!("PATH={}", base.to_str().unwrap()),
    ]);
    let original = server.newest_pane();
    server.run(&["pane", "split", &original, "--direction", "right"]);
    let split = server.newest_pane();
    assert_eq!(server.pane(&split)["plugin_path_dirs"], json!([bin_a]));
    server.run(&["server", "stop"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while server
        .command()
        .args(["pane", "list"])
        .output()
        .unwrap()
        .status
        .success()
    {
        assert!(Instant::now() < deadline, "isolated daemon did not stop");
        std::thread::sleep(Duration::from_millis(50));
    }
    server.run(&["plugin", "disable", "a.alpha"]);
    server.run(&["plugin", "enable", "z.omega"]);
    server.run(&["workspace", "create", "restart-trigger"]);
    assert_eq!(
        server.pane(&server.newest_pane())["plugin_path_dirs"],
        json!([bin_z])
    );
    let list = server.run(&["pane", "list", "--session", "restore-me"]);
    let panes = list["result"]["panes"].as_array().unwrap();
    assert_eq!(panes.len(), 2, "restored pane count: {list}");
    for (index, pane) in panes.iter().enumerate() {
        assert_eq!(pane["plugin_path_dirs"], json!([bin_z]));
        assert_eq!(
            server.path(
                pane["pane_id"].as_str().unwrap(),
                &format!("restored-{index}")
            ),
            vec![bin_z.clone(), base.clone()]
        );
    }
}
