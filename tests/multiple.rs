use aw_calendar_sync::{adapter::synchronize_all, config::CalendarSet, Config};
use chrono::{Duration, Utc};
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn legacy_source_and_new_template_preserve_bucket_identity() {
    let legacy = r#"{"calendar_id":"outlook-work","calendar_file":"work.ics","excluded_busy_statuses":["OOF"],"reconcile":true}"#;
    let set = CalendarSet::from_json(legacy).unwrap();
    assert_eq!(
        set.calendars[0].bucket_id(),
        "aw-calendar-sync_outlook-work"
    );
    let set = CalendarSet::from_json(&CalendarSet::template(false).to_string()).unwrap();
    assert_eq!(
        set.calendars[0].bucket_id(),
        "aw-calendar-sync_outlook-work"
    );
    assert_eq!(set.poll_seconds, 3600);
}
#[test]
fn shared_defaults_and_per_calendar_overrides_are_resolved() {
    let set=CalendarSet::from_json(r#"{
        "excluded_busy_statuses":["OOF"], "reconcile":true, "lookback_days":7,
        "calendars":[
            {"calendar_id":"work","calendar_file":"work.ics"},
            {"calendar_id":"team","calendar_file":"team.ics","excluded_busy_statuses":[],"reconcile":false,"lookback_days":1,"default_timezone":"America/New_York","include_attendees":true},
            {"calendar_id":"inactive","enabled":false}
        ]}"#).unwrap();
    assert_eq!(set.calendars.len(), 2);
    assert_eq!(set.calendars[0].excluded_busy_statuses, vec!["OOF"]);
    assert!(set.calendars[1].excluded_busy_statuses.is_empty());
    assert!(!set.calendars[1].reconcile);
    assert_eq!(set.calendars[1].lookback_days, 1);
    assert!(set.calendars[1].include_attendees);
    assert_eq!(set.calendars[1].default_timezone, "America/New_York");
    assert_ne!(set.calendars[0].bucket_id(), set.calendars[1].bucket_id());
}
#[test]
fn duplicate_ids_and_ambiguous_or_invalid_config_fail_before_sync() {
    for config in [
        r#"{"calendars":[]}"#,
        r#"{"calendars":[{"calendar_id":"work","calendar_file":"work.ics"},{"calendar_id":"WORK","enabled":false}]}"#,
        r#"{"calendar_url":"https://example.invalid/private","calendars":[{"calendar_id":"work","calendar_file":"work.ics"}]}"#,
        r#"{"calendars":[{"calendar_id":"work","calendar_file":"work.ics","unknown":true}]}"#,
        r#"{"poll_seconds":1,"calendars":[{"calendar_id":"work","calendar_file":"work.ics"}]}"#,
    ] {
        assert!(CalendarSet::from_json(config).is_err());
    }
    let err = CalendarSet::from_json(
        r#"{"calendars":[{"calendar_id":"work","calendar_url":123,"secret":"private-link"}]}"#,
    )
    .err()
    .unwrap();
    assert!(!err.contains("private-link"));
}
fn feed() -> String {
    let now = Utc::now();
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:shared-synthetic-uid\r\nSUMMARY:Synthetic meeting\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",(now-Duration::hours(2)).format("%Y%m%dT%H%M%SZ"),(now-Duration::hours(1)).format("%Y%m%dT%H%M%SZ"))
}
#[test]
fn failed_source_does_not_stop_other_sources_or_leak_calendar_content() {
    let root = std::env::temp_dir().join(format!("aw-calendar-multiple-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let bad = root.join("bad.ics");
    let good = root.join("good.ics");
    fs::write(&bad, "<html>login</html>").unwrap();
    fs::write(&good, feed()).unwrap();
    let configs = vec![
        Config {
            calendar_id: "bad".into(),
            calendar_file: Some(bad.to_string_lossy().into()),
            ..Config::default()
        },
        Config {
            calendar_id: "good".into(),
            calendar_file: Some(good.to_string_lossy().into()),
            ..Config::default()
        },
    ];
    let results: Vec<_> = synchronize_all(&configs, Utc::now(), true).collect();
    assert!(results[0].result.is_err());
    assert_eq!(results[1].result.as_ref().unwrap().eligible, 1);
    let config = root.join("config.json");
    fs::write(
        &config,
        json!({"calendars":[
            {"calendar_id":"bad","calendar_file":"bad.ics"},
            {"calendar_id":"good","calendar_file":"good.ics"},
            {"calendar_id":"disabled","enabled":false}
        ]})
        .to_string(),
    )
    .unwrap();
    let validation = Command::new(env!("CARGO_BIN_EXE_aw-calendar-sync"))
        .args(["--config", config.to_str().unwrap(), "--validate-config"])
        .output()
        .unwrap();
    assert!(validation.status.success());
    assert!(String::from_utf8(validation.stdout)
        .unwrap()
        .contains("2 calendars enabled"));
    assert!(!root.join("config.log").exists());
    let output = Command::new(env!("CARGO_BIN_EXE_aw-calendar-sync"))
        .args(["--config", config.to_str().unwrap(), "--dry-run"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let log = String::from_utf8(output.stderr).unwrap();
    assert!(log.contains("calendar=bad"));
    assert!(log.contains("calendar=good dry-run eligible=1"));
    assert!(!log.contains("Synthetic meeting"));
    assert!(!log.contains("calendar=disabled"));
    for file in [
        "config.json",
        "bad.ics",
        "good.ics",
        "config.log",
        "config.lock",
    ] {
        fs::remove_file(root.join(file)).unwrap();
    }
    fs::remove_dir(root).unwrap();
}
