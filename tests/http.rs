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
