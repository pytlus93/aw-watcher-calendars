use aw_calendar_sync::{adapter::synchronize_all, config::CalendarSet};
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration as Wait,
};

#[derive(Default)]
struct State {
    buckets: HashMap<String, Vec<Value>>,
    next_id: u64,
}
struct Server {
    url: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let data = state.clone();
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => {
                        thread::sleep(Wait::from_millis(5));
                        continue;
                    }
                };
                stream.set_read_timeout(Some(Wait::from_secs(3))).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Vec<_> = line.split_whitespace().collect();
                let method = request[0];
                let path = request[1].split('?').next().unwrap();
                let parts: Vec<_> = path.split('/').collect();
                let bucket = parts[4].to_string();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" || header.is_empty() {
                        break;
                    }
                    if let Some((key, val)) = header.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") {
                            length = val.trim().parse().unwrap();
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut state = data.lock().unwrap();
                let (code, response) = match (method, parts.len()) {
                    ("GET", 5) => {
                        if state.buckets.contains_key(&bucket) {
                            (200, json!({"client":"aw-calendar-sync","type":"calendar"}))
                        } else {
                            (404, json!({}))
                        }
                    }
                    ("POST", 5) => {
                        state.buckets.entry(bucket).or_default();
                        (200, json!({}))
                    }
                    ("GET", 6) => (200, json!(state.buckets.get(&bucket).unwrap())),
                    ("POST", 6) => {
                        let mut events: Vec<Value> = serde_json::from_slice(&body).unwrap();
                        for event in &mut events {
                            state.next_id += 1;
                            event["id"] = json!(state.next_id);
                        }
                        state.buckets.get_mut(&bucket).unwrap().extend(events);
                        (200, json!({"inserted":1}))
                    }
                    ("DELETE", 7) => {
                        let id: u64 = parts[6].parse().unwrap();
                        state
                            .buckets
                            .get_mut(&bucket)
                            .unwrap()
                            .retain(|e| e["id"] != id);
                        (200, json!({"success":true}))
                    }
                    ("GET", 7) => {
                        let id: u64 = parts[6].parse().unwrap();
                        match state
                            .buckets
                            .get(&bucket)
                            .unwrap()
                            .iter()
                            .find(|e| e["id"] == id)
                        {
                            Some(e) => (200, e.clone()),
                            None => (404, json!({})),
                        }
                    }
                    _ => (405, json!({})),
                };
                let response = response.to_string();
                write!(stream,"HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
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
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn feed() -> String {
    let now = Utc::now();
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:same-uid\r\nSUMMARY:Synthetic\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",(now-Duration::hours(2)).format("%Y%m%dT%H%M%SZ"),(now-Duration::hours(1)).format("%Y%m%dT%H%M%SZ"))
}
#[test]
fn overlapping_uids_reconciliation_failures_and_removed_sources_are_isolated() {
    let server = Server::start();
    let root = std::env::temp_dir().join(format!("aw-calendars-buckets-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let work = root.join("work.ics");
    let team = root.join("team.ics");
    let content = feed();
    std::fs::write(&work, &content).unwrap();
    std::fs::write(&team, &content).unwrap();
    let settings = json!({"server_url":server.url,"calendars":[
        {"calendar_id":"work","calendar_file":work}, {"calendar_id":"team","calendar_file":team}
    ]});
    let configs = CalendarSet::from_json(&settings.to_string()).unwrap();
    let reports: Vec<_> = synchronize_all(&configs.calendars, Utc::now(), false).collect();
    assert!(reports
        .iter()
        .all(|r| r.result.as_ref().unwrap().inserted == 1));
    std::fs::write(
        &work,
        content.replace("END:VEVENT", "X-MICROSOFT-CDO-BUSYSTATUS:OOF\r\nEND:VEVENT"),
    )
    .unwrap();
    let reports: Vec<_> = synchronize_all(&configs.calendars, Utc::now(), false).collect();
    assert_eq!(reports[0].result.as_ref().unwrap().deleted, 1);
    assert_eq!(reports[1].result.as_ref().unwrap().unchanged, 1);
    assert_eq!(
        server.state.lock().unwrap().buckets["aw-calendar-sync_team"].len(),
        1
    );
    std::fs::write(&work, &content).unwrap();
    synchronize_all(&configs.calendars, Utc::now(), false).for_each(|r| {
        r.result.unwrap();
    });
    std::fs::write(&work, "<html>error</html>").unwrap();
    let reports: Vec<_> = synchronize_all(&configs.calendars, Utc::now(), false).collect();
    assert!(reports[0].result.is_err());
    assert_eq!(reports[1].result.as_ref().unwrap().unchanged, 1);
    assert_eq!(
        server.state.lock().unwrap().buckets["aw-calendar-sync_work"].len(),
        1
    );
    let only_team = CalendarSet::from_json(
        &json!({"server_url":server.url,"calendars":[
            {"calendar_id":"team","calendar_file":team}, {"calendar_id":"work","enabled":false}
        ]})
        .to_string(),
    )
    .unwrap();
    let reports: Vec<_> = synchronize_all(&only_team.calendars, Utc::now(), false).collect();
    assert_eq!(reports.len(), 1);
    reports.into_iter().for_each(|r| {
        r.result.unwrap();
    });
    assert_eq!(
        server.state.lock().unwrap().buckets["aw-calendar-sync_work"].len(),
        1
    );
    std::fs::remove_file(work).unwrap();
    std::fs::remove_file(team).unwrap();
    std::fs::remove_dir(root).unwrap();
}
