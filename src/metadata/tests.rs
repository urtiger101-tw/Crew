use super::*;

fn request(items: &[(&str, Option<&str>)], seq: Option<u64>, ttl_ms: Option<u64>) -> Request {
    Request {
        target: Target::Pane(1),
        source: "hook:test".into(),
        tokens: items
            .iter()
            .map(|(k, v)| ((*k).into(), v.map(str::to_string)))
            .collect(),
        seq,
        ttl_ms,
        title: TextPatch::default(),
        display_agent: TextPatch::default(),
        state_labels: BTreeMap::new(),
        agent: None,
        applies_to_source: None,
        clear_presentation: false,
    }
}

#[test]
fn patches_preserve_other_keys_and_normalize_display_text() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    store
        .apply(
            key,
            &request(
                &[("task", Some("  build\n\t中  ")), ("old", Some("x"))],
                None,
                None,
            ),
            now,
        )
        .unwrap();
    let mut next = request(
        &[("task", Some("next")), ("extra", Some(" y "))],
        None,
        None,
    );
    next.source = "other".into();
    store.apply(key, &next, now).unwrap();
    assert_eq!(
        store.values(key, now),
        BTreeMap::from([
            ("task".into(), "next".into()),
            ("old".into(), "x".into()),
            ("extra".into(), "y".into())
        ])
    );
    let long = "中".repeat(100);
    store
        .apply(
            key,
            &request(
                &[
                    ("old", None),
                    ("extra", Some("\n\t ")),
                    ("long", Some(&long)),
                ],
                None,
                None,
            ),
            now,
        )
        .unwrap();
    let values = store.values(key, now);
    assert_eq!(values.len(), 2);
    assert_eq!(values["long"].chars().count(), 80);
    assert_eq!(values["task"], "next");
}

#[test]
fn ttl_is_per_key_and_expiry_is_reported_once() {
    let now = Instant::now();
    let key = Key::Workspace(2);
    let mut store = Store::default();
    store
        .apply(
            key,
            &request(&[("temporary", Some("one"))], None, Some(10)),
            now,
        )
        .unwrap();
    store
        .apply(key, &request(&[("kept", Some("two"))], None, None), now)
        .unwrap();
    assert_eq!(store.values(key, now + Duration::from_millis(9)).len(), 2);
    let deadline = now + Duration::from_millis(10);
    assert_eq!(
        store.values(key, deadline),
        BTreeMap::from([("kept".into(), "two".into())])
    );
    assert!(
        store.resources[&key].tokens.contains_key("temporary"),
        "reads must leave expiry evidence for sweep"
    );
    assert_eq!(
        store.expire(deadline),
        vec![(key, vec!["temporary".into()])]
    );
    assert!(store.expire(deadline).is_empty());
}

#[test]
fn replacement_refreshes_only_its_own_deadline_and_can_cancel_ttl() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    store
        .apply(
            key,
            &request(
                &[("refresh", Some("old")), ("other", Some("old"))],
                Some(1),
                Some(10),
            ),
            now,
        )
        .unwrap();
    store
        .apply(
            key,
            &request(&[("refresh", Some("new"))], Some(2), Some(30)),
            now + Duration::from_millis(5),
        )
        .unwrap();
    assert_eq!(
        store.expire(now + Duration::from_millis(10)),
        vec![(key, vec!["other".into()])]
    );
    assert_eq!(
        store.values(key, now + Duration::from_millis(10))["refresh"],
        "new"
    );
    store
        .apply(
            key,
            &request(&[("refresh", Some("forever"))], Some(3), None),
            now + Duration::from_millis(20),
        )
        .unwrap();
    assert!(store.expire(now + Duration::from_secs(100)).is_empty());
    assert_eq!(
        store.values(key, now + Duration::from_secs(100))["refresh"],
        "forever"
    );
}

#[test]
fn stale_reports_do_not_refresh_ttl_or_resurrect_cleared_tokens() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    store
        .apply(
            key,
            &request(&[("task", Some("current"))], Some(2), Some(10)),
            now,
        )
        .unwrap();
    for seq in [0, 1, 2] {
        let outcome = store
            .apply(
                key,
                &request(&[("task", Some("stale"))], Some(seq), Some(1000)),
                now + Duration::from_millis(5),
            )
            .unwrap();
        assert_eq!(
            outcome,
            Outcome {
                accepted: false,
                changed: Vec::new()
            }
        );
    }
    store.expire(now + Duration::from_millis(10));
    assert!(
        !store
            .apply(
                key,
                &request(&[("task", Some("late"))], Some(2), None),
                now + Duration::from_millis(11)
            )
            .unwrap()
            .accepted
    );
    store
        .apply(
            key,
            &request(&[("task", Some("next"))], Some(3), None),
            now + Duration::from_millis(11),
        )
        .unwrap();
    store
        .apply(
            key,
            &request(&[("task", None)], Some(4), None),
            now + Duration::from_millis(12),
        )
        .unwrap();
    assert!(
        !store
            .apply(
                key,
                &request(&[("task", Some("late"))], Some(3), None),
                now + Duration::from_millis(13)
            )
            .unwrap()
            .accepted
    );
    assert!(
        store
            .values(key, now + Duration::from_millis(13))
            .is_empty()
    );
}

#[test]
fn key_limit_rejects_atomically_without_consuming_sequence() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    for batch in 0..2 {
        let mut req = request(&[], Some(batch), None);
        req.tokens = (0..16)
            .map(|i| (format!("key{}", i + batch * 16), Some("v".into())))
            .collect();
        store.apply(key, &req, now).unwrap();
    }
    let before = store.resources[&key].clone();
    let error = store
        .apply(
            key,
            &request(
                &[("key0", Some("changed")), ("overflow", Some("x"))],
                Some(2),
                None,
            ),
            now,
        )
        .unwrap_err();
    assert_eq!(error.code, "metadata_key_limit");
    assert_eq!(store.resources[&key], before);
    let outcome = store
        .apply(
            key,
            &request(&[("key0", None), ("replacement", Some("v"))], Some(2), None),
            now,
        )
        .unwrap();
    assert!(outcome.accepted);
    assert_eq!(store.values(key, now).len(), 32);
    assert!(!store.values(key, now).contains_key("key0"));
}

#[test]
fn sequenced_sources_are_bounded_and_survive_clear_and_expiry() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    for source in 0..32 {
        let mut req = request(&[("value", Some("v"))], Some(0), Some(1));
        req.source = format!("hook:{source}");
        store.apply(key, &req, now).unwrap();
    }
    store.expire(now + Duration::from_millis(1));
    let before = store.resources[&key].clone();
    assert_eq!(
        store
            .apply(key, &request(&[("value", Some("x"))], Some(0), None), now)
            .unwrap_err()
            .code,
        "metadata_source_limit"
    );
    assert_eq!(store.resources[&key], before);
    let mut known = request(&[("value", None)], Some(1), None);
    known.source = "hook:0".into();
    assert!(store.apply(key, &known, now).unwrap().accepted);
    assert_eq!(store.resources[&key].sequences.len(), 32);
    // Unsequenced reports do not claim sequence ordering or consume source slots.
    assert!(
        store
            .apply(
                key,
                &request(&[("value", Some("unsequenced"))], None, None),
                now
            )
            .unwrap()
            .accepted
    );
}

#[test]
fn resources_are_isolated_and_closed_resources_release_all_state() {
    let now = Instant::now();
    let mut store = Store::default();
    for key in [Key::Pane(1), Key::Pane(2), Key::Workspace(1)] {
        store
            .apply(key, &request(&[("task", Some("v"))], Some(10), None), now)
            .unwrap();
    }
    store.retain_live(|key| *key == Key::Workspace(1));
    assert_eq!(store.resources.len(), 1);
    assert!(store.values(Key::Pane(1), now).is_empty());
    assert!(
        store
            .apply(
                Key::Pane(1),
                &request(&[("task", Some("new lifetime"))], Some(1), None),
                now
            )
            .unwrap()
            .accepted
    );
    assert_eq!(store.values(Key::Workspace(1), now)["task"], "v");
}

#[test]
fn validation_limits_apply_to_direct_requests_without_partial_effects() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let valid = request(&[("k", Some("v"))], Some(1), None);
    let mut invalid = Vec::new();
    for source in [
        "".to_string(),
        "space here".into(),
        "x".repeat(81),
        "中文".into(),
    ] {
        let mut req = valid.clone();
        req.source = source;
        invalid.push(req);
    }
    for name in ["".to_string(), "a.b".into(), "k".repeat(33), "中文".into()] {
        let mut req = valid.clone();
        req.tokens = BTreeMap::from([(name, Some("v".into()))]);
        invalid.push(req);
    }
    for ttl in [0, MAX_TTL_MS + 1, u64::MAX] {
        let mut req = valid.clone();
        req.ttl_ms = Some(ttl);
        invalid.push(req);
    }
    let mut many = valid.clone();
    many.tokens = (0..17)
        .map(|i| (format!("k{i}"), Some("v".into())))
        .collect();
    invalid.push(many);
    invalid.push(request(&[], None, None));
    let mut store = Store::default();
    for req in invalid {
        assert_eq!(
            store.apply(key, &req, now).unwrap_err().code,
            "invalid_argument"
        );
        assert!(store.resources.is_empty());
    }
    for ttl in [1, MAX_TTL_MS] {
        assert!(
            store
                .apply(key, &request(&[("k", Some("v"))], None, Some(ttl)), now)
                .is_ok()
        );
    }
}

#[test]
fn cli_preserves_literals_and_rejects_ambiguous_options() {
    let parse = |group: &str, args: &[&str]| {
        cli::parse(
            group,
            &args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
        )
    };
    let ApiRequest::Metadata(req) = parse(
        "workspace",
        &[
            "project.alpha:中文",
            "--source=hook:test",
            "--token=k=a=b c",
            "--clear-token",
            "old",
            "--seq",
            "0",
            "--ttl-ms",
            "1",
        ],
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(req.target, Target::Workspace("project.alpha:中文".into()));
    assert_eq!(req.tokens["k"].as_deref(), Some("a=b c"));
    assert_eq!(req.tokens["old"], None);
    assert_eq!(req.seq, Some(0));
    for args in [
        vec!["p1", "--source", "hook", "--token", "k=x", "--token", "k=y"],
        vec![
            "p1",
            "--source",
            "hook",
            "--token",
            "k=x",
            "--clear-token",
            "k",
        ],
        vec!["p1", "--source", "a", "--source", "b", "--token", "k=x"],
        vec!["p1", "--source", "hook", "--token", "k=x", "--seq", "-1"],
        vec!["p1", "--source", "hook", "--token", "k=x", "--seq", "+1"],
        vec!["p1", "--source", "hook", "--token", "k=x", "--ttl-ms", "0"],
        vec!["p1", "--source", "hook", "--unsupported", "not supported"],
        vec!["p01", "--source", "hook", "--token", "k=x"],
        vec!["p1", "p2", "--source", "hook", "--token", "k=x"],
        vec!["p1", "--source", "hook"],
    ] {
        assert!(parse("pane", &args).is_err(), "{args:?}");
    }
}

use crate::api::Request as ApiRequest;

#[test]
fn patch_after_expiry_cannot_resurrect_omitted_labels_or_guards_before_sweep() {
    let now = Instant::now();
    let later = now + Duration::from_millis(11);
    let key = Key::Pane(1);
    for sweep_first in [false, true] {
        let mut store = Store::default();
        let mut req = request(&[], Some(1), Some(10));
        req.title = TextPatch::set("old title".into());
        req.display_agent = TextPatch::set("old label".into());
        req.agent = Some("old-agent".into());
        store.apply(key, &req, now).unwrap();
        if sweep_first {
            store.expire(later);
        }
        let mut fresh = request(&[], Some(2), None);
        fresh.title = TextPatch::set("new title".into());
        store.apply(key, &fresh, later).unwrap();
        let value = store.presentation(key, later, None, None);
        assert_eq!(value.title.as_deref(), Some("new title"));
        assert_eq!(value.display_agent, None);
    }
}

#[test]
fn handler_validates_direct_api_and_expiry_without_starting_a_server() {
    let mut st = crate::server::test_state();
    st.sessions.push(crate::server::Session {
        id: 1,
        name: "project".into(),
        windows: Vec::new(),
        active: 0,
        cols: 80,
        rows: 24,
        sidebar: false,
        cwd: None,
        env: BTreeMap::new(),
        last_used: Instant::now(),
    });
    let shared = std::sync::Arc::new(std::sync::Mutex::new(st));
    let mut req = request(&[("task", Some("v"))], Some(7), Some(10));
    req.target = Target::Workspace("w1".into());
    let mut bad = req.clone();
    bad.ttl_ms = Some(0);
    assert_eq!(
        crate::api::handle(&shared, ApiRequest::Metadata(Box::new(bad)))
            .unwrap_err()
            .code,
        "invalid_argument"
    );
    assert!(lock(&shared).metadata.resources.is_empty());
    let Reply::Json(result) =
        crate::api::handle(&shared, ApiRequest::Metadata(Box::new(req.clone()))).unwrap()
    else {
        panic!()
    };
    assert_eq!(result["accepted"], true);
    let Reply::Json(stale) =
        crate::api::handle(&shared, ApiRequest::Metadata(Box::new(req))).unwrap()
    else {
        panic!()
    };
    assert_eq!(stale["accepted"], false);
    {
        let mut st = lock(&shared);
        sweep(&mut st, Instant::now() + Duration::from_millis(20));
        assert!(
            st.metadata
                .values(Key::Workspace(1), Instant::now())
                .is_empty()
        );
        st.sessions.clear();
        sweep(&mut st, Instant::now());
        assert!(st.metadata.resources.is_empty());
    }
    let Reply::Json(events) = crate::events::handle(
        &shared,
        crate::events::Request::Read {
            after: 0,
            limit: 512,
            epoch: None,
            timeout_ms: 0,
        },
    )
    .unwrap() else {
        panic!()
    };
    let events = events["events"].as_array().unwrap();
    assert_eq!(
        events.len(),
        2,
        "one report and one expiry; stale reports do not emit"
    );
    assert_eq!(events[0]["kind"], "workspace.metadata_updated");
    assert_eq!(events[0]["data"]["reason"], "report");
    assert_eq!(events[1]["data"]["reason"], "expired");
}

#[test]
fn raw_metadata_schema_rejects_unknown_fields() {
    let req = request(&[("task", Some("v"))], None, None);
    let mut value = serde_json::to_value(&req).unwrap();
    value["unknown_presentation_field"] = json!("unsupported");
    assert!(serde_json::from_value::<Request>(value).is_err());
}

#[test]
fn presentation_null_clear_and_omission_have_distinct_wire_semantics() {
    let mut req = request(&[], Some(1), None);
    req.title = TextPatch::set("First".into());
    let mut value = serde_json::to_value(&req).unwrap();
    assert!(value.get("display_agent").is_none());
    value["title"] = serde_json::Value::Null;
    let decoded: Request = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.title, TextPatch::clear());
    assert!(decoded.display_agent.missing());
}

#[test]
fn presentation_patches_keep_omitted_fields_and_do_not_change_tokens() {
    let now = Instant::now();
    let mut store = Store::default();
    let key = Key::Pane(1);
    let mut req = request(&[("task", Some("work"))], Some(1), None);
    req.title = TextPatch::set("  Title\n中文\u{202e} ".into());
    req.display_agent = TextPatch::set("Team A".into());
    req.state_labels
        .insert("working".into(), Some("Building".into()));
    assert!(store.apply(key, &req, now).unwrap().accepted);
    let original = store.presentation(key, now, None, None);
    assert_eq!(original.title.as_deref(), Some("Title中文"));
    let mut clear = request(&[], Some(2), None);
    clear.title = TextPatch::clear();
    store.apply(key, &clear, now).unwrap();
    let view = store.presentation(key, now, None, None);
    assert_eq!(view.title, None);
    assert_eq!(view.display_agent, original.display_agent);
    assert_eq!(view.state_labels["working"], "Building");
    assert_eq!(store.values(key, now)["task"], "work");
    clear.title = TextPatch::default();
    clear.clear_presentation = true;
    clear.seq = Some(3);
    store.apply(key, &clear, now).unwrap();
    assert_eq!(
        store.presentation(key, now, None, None),
        presentation::Presentation::default()
    );
    assert_eq!(store.values(key, now)["task"], "work");
    assert!(!store.apply(key, &req, now).unwrap().accepted);
}

#[test]
fn guarded_presentation_is_hidden_but_tokens_remain_independent() {
    let now = Instant::now();
    let mut store = Store::default();
    let key = Key::Pane(1);
    let mut req = request(&[("task", Some("shared"))], Some(1), None);
    req.title = TextPatch::set("Guarded".into());
    req.agent = Some("codex".into());
    req.applies_to_source = Some("hook:codex".into());
    store.apply(key, &req, now).unwrap();
    for (agent, source) in [
        (None, None),
        (Some("claude"), Some("hook:codex")),
        (Some("codex"), Some("other")),
        (Some("codex"), None),
    ] {
        assert!(store.presentation(key, now, agent, source).title.is_none());
        assert_eq!(store.values(key, now)["task"], "shared");
    }
    assert_eq!(
        store
            .presentation(key, now, Some("codex"), Some("hook:codex"))
            .title
            .as_deref(),
        Some("Guarded")
    );
}

#[test]
fn presentation_source_ttl_and_sequence_are_not_refreshed_by_token_only_reports() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    let mut req = request(&[], Some(10), Some(10));
    req.title = TextPatch::set("short-lived".into());
    store.apply(key, &req, now).unwrap();
    store
        .apply(key, &request(&[("kept", Some("yes"))], Some(11), None), now)
        .unwrap();
    let end = now + Duration::from_millis(10);
    assert!(store.presentation(key, end, None, None).title.is_none());
    assert_eq!(store.expire(end), vec![(key, vec!["@presentation".into()])]);
    assert!(store.expire(end).is_empty());
    assert!(!store.apply(key, &req, end).unwrap().accepted);
    assert_eq!(store.values(key, end)["kept"], "yes");
}

#[test]
fn guard_only_patches_and_clearing_last_field_keep_both_guards_until_explicit_clear() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    let mut guard = request(&[], Some(10), None);
    guard.agent = Some("codex".into());
    guard.applies_to_source = Some("hook:codex".into());
    assert_eq!(
        store.apply(key, &guard, now).unwrap().changed,
        vec!["@presentation"]
    );
    assert_eq!(store.resources[&key].presentations.len(), 1);
    let mut title = request(&[], Some(11), None);
    title.title = TextPatch::set("First".into());
    store.apply(key, &title, now).unwrap();
    let mut clear = request(&[], Some(12), None);
    clear.title = TextPatch::clear();
    store.apply(key, &clear, now).unwrap();
    assert_eq!(
        store.resources[&key].presentations["hook:test"].value,
        presentation::Presentation::default()
    );
    title.seq = Some(13);
    title.title = TextPatch::set("Later".into());
    store.apply(key, &title, now).unwrap();
    for (agent, source) in [
        (Some("claude"), Some("hook:codex")),
        (Some("codex"), Some("other")),
        (None, None),
    ] {
        assert!(store.presentation(key, now, agent, source).title.is_none());
    }
    assert_eq!(
        store
            .presentation(key, now, Some("codex"), Some("hook:codex"))
            .title
            .as_deref(),
        Some("Later")
    );
    guard.seq = Some(12);
    guard.agent = Some("claude".into());
    assert!(!store.apply(key, &guard, now).unwrap().accepted);
    assert_eq!(
        store.resources[&key].presentations["hook:test"]
            .agent
            .as_deref(),
        Some("codex")
    );
    let mut remove = request(&[], Some(14), None);
    remove.clear_presentation = true;
    store.apply(key, &remove, now).unwrap();
    title.seq = Some(15);
    store.apply(key, &title, now).unwrap();
    assert_eq!(
        store.presentation(key, now, None, None).title.as_deref(),
        Some("Later")
    );
}

#[test]
fn guard_only_ttl_expires_without_token_refresh_and_without_reviving_before_sweep() {
    let now = Instant::now();
    let end = now + Duration::from_millis(10);
    let key = Key::Pane(1);
    for sweep_first in [false, true] {
        let mut store = Store::default();
        let mut guard = request(&[], Some(1), Some(10));
        guard.agent = Some("codex".into());
        store.apply(key, &guard, now).unwrap();
        store
            .apply(key, &request(&[("task", Some("keep"))], Some(2), None), now)
            .unwrap();
        assert_eq!(
            store.resources[&key].presentations["hook:test"].deadline,
            Some(end)
        );
        if sweep_first {
            assert_eq!(store.expire(end), vec![(key, vec!["@presentation".into()])]);
        }
        let mut title = request(&[], Some(3), None);
        title.title = TextPatch::set("Fresh".into());
        store.apply(key, &title, end).unwrap();
        assert_eq!(
            store.presentation(key, end, None, None).title.as_deref(),
            Some("Fresh")
        );
        assert!(!store.apply(key, &guard, end).unwrap().accepted);
        assert_eq!(store.values(key, end)["task"], "keep");
    }
}

#[test]
fn guard_only_sources_are_bounded_and_clear_or_expiry_releases_capacity_atomically() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    for n in 0..MAX_SOURCES {
        let mut guard = request(&[], None, Some(10));
        guard.source = format!("guard:{n}");
        guard.agent = Some("codex".into());
        store.apply(key, &guard, now).unwrap();
    }
    let before = store.resources[&key].clone();
    let mut overflow = request(&[("task", Some("overflow"))], Some(0), None);
    overflow.agent = Some("claude".into());
    assert_eq!(
        store.apply(key, &overflow, now).unwrap_err().code,
        "metadata_source_limit"
    );
    assert_eq!(store.resources[&key], before);
    let mut clear = request(&[], None, None);
    clear.source = "guard:0".into();
    clear.clear_presentation = true;
    store.apply(key, &clear, now).unwrap();
    assert!(store.apply(key, &overflow, now).unwrap().accepted);
    assert_eq!(store.resources[&key].presentations.len(), MAX_SOURCES);
    store.expire(now + Duration::from_millis(10));
    assert_eq!(store.resources[&key].presentations.len(), 1);
    overflow.source = "after-expiry".into();
    assert!(
        store
            .apply(key, &overflow, now + Duration::from_millis(10))
            .unwrap()
            .accepted
    );
}

#[test]
fn presentation_sources_fall_back_after_newer_source_clears_or_expires() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    let mut first = request(&[], None, None);
    first.title = TextPatch::set("Older".into());
    store.apply(key, &first, now).unwrap();
    let mut next = first.clone();
    next.source = "other".into();
    next.title = TextPatch::set("Newer".into());
    next.ttl_ms = Some(5);
    store.apply(key, &next, now).unwrap();
    assert_eq!(
        store.presentation(key, now, None, None).title.as_deref(),
        Some("Newer")
    );
    assert_eq!(
        store
            .presentation(key, now + Duration::from_millis(5), None, None)
            .title
            .as_deref(),
        Some("Older")
    );
}

#[test]
fn invalid_presentation_is_atomic_and_unsequenced_sources_are_bounded() {
    let now = Instant::now();
    let key = Key::Pane(1);
    let mut store = Store::default();
    for n in 0..32 {
        let mut req = request(&[], None, None);
        req.source = format!("source{n}");
        req.title = TextPatch::set("title".into());
        store.apply(key, &req, now).unwrap();
    }
    let mut overflow = request(&[("task", Some("must-not-appear"))], Some(0), None);
    overflow.title = TextPatch::set("overflow".into());
    assert_eq!(
        store.apply(key, &overflow, now).unwrap_err().code,
        "metadata_source_limit"
    );
    assert!(store.values(key, now).is_empty());
    assert!(!store.resources[&key].sequences.contains_key("hook:test"));
    overflow.source = "source0".into();
    overflow
        .state_labels
        .insert("bogus".into(), Some("invalid".into()));
    assert_eq!(
        store.apply(key, &overflow, now).unwrap_err().code,
        "invalid_argument"
    );
    overflow.state_labels.clear();
    overflow.target = Target::Workspace("w1".into());
    overflow.display_agent = TextPatch::set("not-a-workspace-field".into());
    assert!(overflow.validate().is_err());
}

#[test]
fn cli_presentation_patches_and_ambiguities() {
    let parse = |group, args: &[&str]| {
        cli::parse(group, &args.iter().map(|s| (*s).into()).collect::<Vec<_>>())
    };
    let ApiRequest::Metadata(req) = parse(
        "pane",
        &[
            "p1",
            "--source",
            "display",
            "--title",
            "A",
            "--display-agent",
            "B",
            "--state-label",
            "working=C",
            "--agent",
            "codex",
            "--applies-to-source",
            "hook:test",
        ],
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(req.title, TextPatch::set("A".into()));
    assert_eq!(req.state_labels["working"].as_deref(), Some("C"));
    for args in [
        vec!["--title", "a", "--clear-title"],
        vec!["--clear-title", "--title", "a"],
        vec!["--clear-presentation", "--display-agent", "a"],
        vec!["--state-label", "idle=a", "--clear-state-label", "idle"],
        vec!["--clear-title=x"],
        vec!["--state-label", "bogus=a"],
    ] {
        let mut all = vec!["p1", "--source", "x"];
        all.extend(args);
        assert!(parse("pane", &all).is_err(), "{all:?}");
    }
    assert!(parse("workspace", &["w1", "--source", "x", "--title", "Work"]).is_ok());
    assert!(
        parse(
            "workspace",
            &["w1", "--source", "x", "--display-agent", "No"]
        )
        .is_err()
    );
}
