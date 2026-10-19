use aw_calendar_sync::{adapter::synchronize, Config, Window};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

#[derive(Default)]
struct State {
    bucket: bool,
    events: Vec<Value>,
    methods: Vec<String>,
    lose_reply: bool,
    fail_insert: bool,
    count_reply: bool,
    next_id: u64,
    fail_delete: bool,
    bucket_race: bool,
    fail_create: bool,
    outage: bool,
    insert_304: bool,
    absent_lookup_500: bool,
}
struct Mock {
    url: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Mock {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let s = state.clone();
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let words: Vec<_> = line.split_whitespace().collect();
                let method = words[0].to_string();
                let path = words[1].to_string();
                let mut length = 0;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    if let Some((name, v)) = h.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = v.trim().parse::<usize>().unwrap();
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut state = s.lock().unwrap();
                state.methods.push(method.clone());
                let (code, value) = match (method.as_str(), path.contains("/events")) {
                    _ if state.outage => (503, json!({})),
                    ("POST", false) if state.fail_create => (503, json!({})),
                    ("POST", false) if state.bucket_race => {
                        state.bucket = true;
                        (304, json!({}))
                    }
                    ("POST", true) if state.insert_304 => (304, json!({})),
                    ("DELETE", true) if state.fail_delete => (503, json!({})),
                    ("DELETE", true) => {
                        let id: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                        state.events.retain(|event| event["id"] != id);
                        (200, json!({"success":true}))
                    }
                    ("GET", true) if path.contains("/events/") => {
                        let id: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                        match state.events.iter().find(|e| e["id"] == id) {
                            Some(e) => (200, e.clone()),
                            None if state.absent_lookup_500 => (500, json!({})),
                            None => (404, json!({})),
                        }
                    }
                    ("GET", false) if !state.bucket => (404, json!({})),
                    ("GET", false) => (200, json!({"client":"aw-calendar-sync","type":"calendar"})),
                    ("POST", false) => {
                        state.bucket = true;
                        (200, json!({}))
                    }
                    ("GET", true) => (200, json!(state.events)),
                    ("POST", true) if state.fail_insert => (503, json!({})),
                    ("POST", true) => {
                        let mut events: Vec<Value> = serde_json::from_slice(&body).unwrap();
                        for e in &mut events {
                            state.next_id += 1;
                            e["id"] = json!(state.next_id);
                            state.events.push(e.clone());
                        }
                        if state.lose_reply {
                            state.lose_reply = false;
                            drop(state);
                            continue;
                        }
                        (
                            200,
                            if state.count_reply {
                                json!({"inserted":events.len()})
                            } else {
                                json!(events)
                            },
                        )
                    }
                    _ => (405, json!({})),
                };
                let body = value.to_string();
                write!(stream,"HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        Self {
            url,
            state,
            stop,
            worker: Some(worker),
        }
    }
}

#[test]
fn count_acknowledgement_is_verified_by_readback_without_duplicate_post() {
    let mock = Mock::start();
    mock.state.lock().unwrap().count_reply = true;
    let (config, path) = fixture(&mock, "count-reply");
    std::fs::write(&path, feed("Original")).unwrap();
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 1);
    assert_eq!(synchronize(&config, window(), false).unwrap().unchanged, 1);
    assert_eq!(mock.state.lock().unwrap().events.len(), 1);
    std::fs::remove_file(path).unwrap();
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn window() -> Window {
    Window {
        start: "2026-10-05T00:00:00Z".parse().unwrap(),
        end: "2026-10-05T12:00:00Z".parse().unwrap(),
    }
}
fn fixture(mock: &Mock, name: &str) -> (Config, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "aw-calendar-sync-{}-{name}.ics",
        std::process::id()
    ));
    let config = Config {
        calendar_file: Some(path.to_string_lossy().into()),
        server_url: mock.url.clone(),
        ..Config::default()
    };
    (config, path)
}
fn feed(title: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:synthetic\r\nSUMMARY:{title}\r\nDTSTART:20261005T090000Z\r\nDTEND:20261005T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n")
}

#[test]
fn repeated_import_edits_and_cancellations_never_delete_or_duplicate() {
    let mock = Mock::start();
    let (mut config, path) = fixture(&mock, "repeated");
    config.reconcile = false;
    std::fs::write(&path, feed("Original")).unwrap();
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 1);
    assert_eq!(synchronize(&config, window(), false).unwrap().unchanged, 1);
    std::fs::write(&path, feed("Changed")).unwrap();
    assert_eq!(synchronize(&config, window(), false).unwrap().changed, 1);
    std::fs::write(&path,"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:synthetic\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n").unwrap();
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 0);
    let state = mock.state.lock().unwrap();
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.events[0]["data"]["title"], "Original");
    assert!(state.methods.iter().all(|m| m == "GET" || m == "POST"));
    drop(state);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn uncertain_insert_response_is_recovered_from_aw() {
    let mock = Mock::start();
    mock.state.lock().unwrap().lose_reply = true;
    let (config, path) = fixture(&mock, "lost");
    std::fs::write(&path, feed("Original")).unwrap();
    assert!(synchronize(&config, window(), false).is_err());
    assert_eq!(synchronize(&config, window(), false).unwrap().unchanged, 1);
    assert_eq!(mock.state.lock().unwrap().events.len(), 1);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn server_failure_retries_without_data_loss() {
    let mock = Mock::start();
    mock.state.lock().unwrap().fail_insert = true;
    let (config, path) = fixture(&mock, "retry");
    std::fs::write(&path, feed("Original")).unwrap();
    assert!(synchronize(&config, window(), false).is_err());
    assert!(mock.state.lock().unwrap().events.is_empty());
    mock.state.lock().unwrap().fail_insert = false;
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 1);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn dry_run_and_invalid_calendar_never_contact_aw() {
    let mock = Mock::start();
    let (config, path) = fixture(&mock, "dry");
    std::fs::write(&path, feed("Original")).unwrap();
    assert_eq!(synchronize(&config, window(), true).unwrap().eligible, 1);
    std::fs::write(&path, "<html>Sign in</html>").unwrap();
    assert!(synchronize(&config, window(), false).is_err());
    assert!(mock.state.lock().unwrap().methods.is_empty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reconciliation_replaces_then_filters_and_preserves_old_history() {
    let mock = Mock::start();
    let (config, path) = fixture(&mock, "reconcile");
    std::fs::write(&path, feed("Original")).unwrap();
    synchronize(&config, window(), false).unwrap();
    std::fs::write(&path, feed("Changed")).unwrap();
    let report = synchronize(&config, window(), false).unwrap();
    assert_eq!(report.inserted, 1);
    assert_eq!(report.deleted, 1);
    assert_eq!(report.changed, 1);
    assert_eq!(mock.state.lock().unwrap().events.len(), 1);
    let oof = feed("Changed").replace("END:VEVENT", "X-MICROSOFT-CDO-BUSYSTATUS:OOF\r\nEND:VEVENT");
    std::fs::write(&path, oof).unwrap();
    assert_eq!(synchronize(&config, window(), false).unwrap().deleted, 1);
    assert!(mock.state.lock().unwrap().events.is_empty());
    std::fs::write(&path, feed("Original")).unwrap();
    synchronize(&config, window(), false).unwrap();
    std::fs::write(&path, "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n").unwrap();
    let next = Window {
        start: "2026-10-06T00:00:00Z".parse().unwrap(),
        end: "2026-10-06T12:00:00Z".parse().unwrap(),
    };
    assert_eq!(synchronize(&config, next, false).unwrap().deleted, 0);
    assert_eq!(mock.state.lock().unwrap().events.len(), 1);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn failed_deletion_is_recovered_without_reinserting_replacement() {
    let mock = Mock::start();
    let (config, path) = fixture(&mock, "delete-retry");
    std::fs::write(&path, feed("Original")).unwrap();
    synchronize(&config, window(), false).unwrap();
    std::fs::write(&path, feed("Changed")).unwrap();
    mock.state.lock().unwrap().fail_delete = true;
    assert!(synchronize(&config, window(), false).is_err());
    assert_eq!(mock.state.lock().unwrap().events.len(), 2);
    mock.state.lock().unwrap().fail_delete = false;
    let report = synchronize(&config, window(), false).unwrap();
    assert_eq!(report.inserted, 0);
    assert_eq!(report.deleted, 1);
    assert_eq!(mock.state.lock().unwrap().events.len(), 1);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn existing_bucket_race_304_and_producer_restart_resume_delivery() {
    let mock = Mock::start();
    mock.state.lock().unwrap().bucket_race = true;
    let (config, path) = fixture(&mock, "bucket-race");
    std::fs::write(&path, feed("Original")).unwrap();
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 1);
    // synchronize constructs a fresh adapter each time: producer restart/replay.
    assert_eq!(synchronize(&config, window(), false).unwrap().unchanged, 1);
    assert_eq!(mock.state.lock().unwrap().events.len(), 1);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn genuine_creation_failure_and_non_creation_304_do_not_count_as_success() {
    let mock = Mock::start();
    let (config, path) = fixture(&mock, "bad-status");
    std::fs::write(&path, feed("Original")).unwrap();
    mock.state.lock().unwrap().fail_create = true;
    assert!(synchronize(&config, window(), false)
        .err()
        .unwrap()
        .contains("HTTP 503"));
    assert!(mock.state.lock().unwrap().events.is_empty());
    mock.state.lock().unwrap().fail_create = false;
    mock.state.lock().unwrap().insert_304 = true;
    assert!(synchronize(&config, window(), false).is_err());
    assert!(mock.state.lock().unwrap().events.is_empty());
    mock.state.lock().unwrap().insert_304 = false;
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 1);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn delayed_server_and_outage_preserve_intervals_and_resume_actual_inserts() {
    let mock = Mock::start();
    let (config, path) = fixture(&mock, "outage");
    let two = feed("First").replace("END:VCALENDAR", "BEGIN:VEVENT\r\nUID:second\r\nSUMMARY:Second\r\nDTSTART:20261005T110000Z\r\nDTEND:20261005T113000Z\r\nEND:VEVENT\r\nEND:VCALENDAR");
    std::fs::write(&path, &two).unwrap();
    mock.state.lock().unwrap().outage = true;
    assert!(synchronize(&config, window(), false).is_err());
    mock.state.lock().unwrap().outage = false;
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 2);
    let saved = mock.state.lock().unwrap().events.clone();
    assert_eq!(saved[0]["timestamp"], "2026-10-05T09:00:00Z");
    assert_eq!(saved[0]["duration"], 3600.0);
    assert_eq!(saved[1]["timestamp"], "2026-10-05T11:00:00Z");
    assert_eq!(saved[1]["duration"], 1800.0);
    mock.state.lock().unwrap().outage = true;
    assert!(synchronize(&config, window(), false).is_err());
    mock.state.lock().unwrap().outage = false;
    assert_eq!(synchronize(&config, window(), false).unwrap().unchanged, 2);
    // Simulate restart with lost server storage, then verify actual redelivery.
    mock.state.lock().unwrap().bucket = false;
    mock.state.lock().unwrap().events.clear();
    assert_eq!(synchronize(&config, window(), false).unwrap().inserted, 2);
    let replayed = mock.state.lock().unwrap().events.clone();
    for (original, replay) in saved.iter().zip(replayed.iter()) {
        assert_eq!(original["timestamp"], replay["timestamp"]);
        assert_eq!(original["duration"], replay["duration"]);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn rust_absent_lookup_500_requires_independent_deletion_confirmation() {
    let mock = Mock::start();
    mock.state.lock().unwrap().absent_lookup_500 = true;
    let (config, path) = fixture(&mock, "rust-delete");
    std::fs::write(&path, feed("Original")).unwrap();
    synchronize(&config, window(), false).unwrap();
    std::fs::write(&path, "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n").unwrap();
    assert_eq!(synchronize(&config, window(), false).unwrap().deleted, 1);
    assert!(mock.state.lock().unwrap().events.is_empty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn privacy_mode_change_reconciles_saved_payload_and_replays_without_duplicates() {
    let mock = Mock::start();
    let (mut config, path) = fixture(&mock, "privacy");
    config.include_attendees = true;
    let feed = feed("Private topic").replace(
        "END:VEVENT",
        "ATTENDEE:mailto:synthetic@example.invalid\r\nEND:VEVENT",
    );
    std::fs::write(&path, feed).unwrap();
    synchronize(&config, window(), false).unwrap();
    assert!(mock.state.lock().unwrap().events[0]["data"]
        .get("attendees")
        .is_some());
    config.payload_mode = aw_calendar_sync::PayloadMode::Strict;
    config.include_summary = false;
    let report = synchronize(&config, window(), false).unwrap();
    assert_eq!(report.inserted, 1);
    assert_eq!(report.deleted, 1);
    assert_eq!(synchronize(&config, window(), false).unwrap().unchanged, 1);
    let state = mock.state.lock().unwrap();
    assert_eq!(state.events.len(), 1);
    let saved = state.events[0].to_string();
    for forbidden in [
        "Private topic",
        "attendees",
        "all_day",
        "status",
        "example.invalid",
    ] {
        assert!(!saved.contains(forbidden));
    }
    drop(state);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn selected_scopes_insert_replay_replace_and_remove_not_yet_completed_intervals() {
    use aw_calendar_sync::ImportScope;
    use chrono::{Duration, Utc};
    for (scope, name) in [
        (ImportScope::Started, "running-scope"),
        (ImportScope::CurrentDay, "day-scope"),
    ] {
        let mock = Mock::start();
        let (mut config, path) = fixture(&mock, name);
        config.import_scope = scope;
        config.default_timezone = "UTC".into();
        let now = Utc::now();
        let window = config.window(now).unwrap();
        let start = if scope == ImportScope::Started {
            now - Duration::minutes(15)
        } else {
            window.end - Duration::minutes(1)
        };
        let end = start + Duration::hours(1);
        assert!(end > now);
        let text=format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:synthetic-scope\r\nSUMMARY:Original\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",start.format("%Y%m%dT%H%M%SZ"),end.format("%Y%m%dT%H%M%SZ"));
        std::fs::write(&path, &text).unwrap();
        assert_eq!(synchronize(&config, window, false).unwrap().inserted, 1);
        assert_eq!(synchronize(&config, window, false).unwrap().unchanged, 1);
        assert_eq!(mock.state.lock().unwrap().events[0]["duration"], 3600.0);
        std::fs::write(&path, text.replace("SUMMARY:Original", "SUMMARY:Changed")).unwrap();
        let changed = synchronize(&config, window, false).unwrap();
        assert_eq!((changed.inserted, changed.deleted), (1, 1));
        std::fs::write(&path, "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n").unwrap();
        assert_eq!(synchronize(&config, window, false).unwrap().deleted, 1);
        assert!(mock.state.lock().unwrap().events.is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
