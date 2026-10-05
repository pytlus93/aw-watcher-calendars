//! AW adapter with bounded reconciliation of its own calendar bucket.
use crate::{
    parse_calendar, plan_import, plan_reconcile, CalendarEvent, Config, Plan, Result, StoredEvent,
    Window,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::{fs::File, io::Read, time::Duration};

const MAX_FEED: u64 = 16 * 1024 * 1024;
const MAX_AW: u64 = 64 * 1024 * 1024;
const MAX_EXISTING: usize = 50000;

fn read_limited(reader: impl Read, limit: u64) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "response read failed")?;
    if bytes.len() as u64 > limit {
        return Err("response size limit exceeded".into());
    }
    String::from_utf8(bytes).map_err(|_| "expected UTF-8 text".into())
}
fn http_error(error: ureq::Error, action: &str) -> String {
    // ureq errors contain the secret ICS URL; never display their Debug/Display.
    match error {
        ureq::Error::Status(code, _) => format!("{action}: HTTP {code}"),
        ureq::Error::Transport(_) => format!("{action}: connection failed or timed out"),
    }
}
fn json_response(response: ureq::Response) -> Result<Value> {
    let text = read_limited(response.into_reader(), MAX_AW)?;
    serde_json::from_str(&text).map_err(|_| "invalid AW JSON response".into())
}

pub fn load_feed(config: &Config) -> Result<String> {
    if let Some(path) = &config.calendar_file {
        return read_limited(
            File::open(path).map_err(|_| "cannot open calendar file")?,
            MAX_FEED,
        );
    }
    let url = config
        .calendar_url
        .as_ref()
        .ok_or("no calendar source configured")?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .redirects(0)
        .build();
    // Outlook published links work directly; redirects are rejected to keep bearer URLs private.
    let response = agent
        .get(url)
        .set("User-Agent", "aw-calendar-sync/0.1.0")
        .set("Accept", "text/calendar")
        .call()
        .map_err(|e| http_error(e, "calendar download"))?;
    if response.status() != 200 {
        return Err("calendar download did not return HTTP 200".into());
    }
    read_limited(response.into_reader(), MAX_FEED)
}

pub struct AwAdapter {
    agent: ureq::Agent,
    base: String,
    bucket: String,
    calendar_id: String,
}
impl AwAdapter {
    pub fn new(config: &Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(15))
                .redirects(0)
                .try_proxy_from_env(false)
                .build(),
            base: config.server_url.trim_end_matches('/').into(),
            bucket: config.bucket_id(),
            calendar_id: config.calendar_id.clone(),
        })
    }
    fn url(&self, tail: &str) -> String {
        format!("{}/api/0/buckets/{}{}", self.base, self.bucket, tail)
    }
    pub fn ensure_bucket(&self) -> Result<()> {
        let metadata = match self.agent.get(&self.url("")).call() {
            Ok(r) => json_response(r)?,
            Err(ureq::Error::Status(404, _)) => {
                self.agent.post(&self.url("")).send_json(json!({
                    "client":"aw-calendar-sync", "type":"calendar",
                    "hostname": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into())
                })).map_err(|e| http_error(e,"create calendar bucket"))?;
                json_response(
                    self.agent
                        .get(&self.url(""))
                        .call()
                        .map_err(|e| http_error(e, "read bucket metadata"))?,
                )?
            }
            Err(e) => return Err(http_error(e, "read bucket metadata")),
        };
        if metadata["client"] != "aw-calendar-sync" || metadata["type"] != "calendar" {
            return Err("bucket belongs to another client or has a different type; choose a new calendar_id".into());
        }
        Ok(())
    }
    pub fn existing(&self) -> Result<Vec<CalendarEvent>> {
        Ok(self
            .records()?
            .into_iter()
            .map(|stored| stored.event)
            .collect())
    }
    pub fn records(&self) -> Result<Vec<StoredEvent>> {
        // Read ALL retained records to recognize moved events, not just today's timestamps.
        // Fail at a cap rather than trusting a partial deduplication view.
        let response = self
            .agent
            .get(&self.url("/events"))
            .query("limit", &(MAX_EXISTING + 1).to_string())
            .call()
            .map_err(|e| http_error(e, "read calendar events"))?;
        let data = json_response(response)?;
        let values = data.as_array().ok_or("expected AW event array")?;
        if values.len() > MAX_EXISTING {
            return Err("retained event limit exceeded; no imports performed".into());
        }
        let mut events = Vec::new();
        for value in values {
            let stored: StoredEvent = serde_json::from_value(value.clone())
                .map_err(|_| "calendar bucket has incompatible event data")?;
            let event = &stored.event;
            if event.data.schema_version != 1 || event.data.calendar_id != self.calendar_id {
                return Err("calendar bucket schema or source identity mismatch".into());
            }
            events.push(stored);
        }
        Ok(events)
    }
    pub fn insert(&self, event: &CalendarEvent) -> Result<()> {
        // One POST at a time. After any uncertainty, the next poll reads AW again.
        let response = self
            .agent
            .post(&self.url("/events"))
            .send_json(json!([event]))
            .map_err(|e| http_error(e, "insert calendar event"))?;
        let result = json_response(response);
        if result
            .as_ref()
            .ok()
            .and_then(Value::as_array)
            .is_some_and(|events| {
                events.len() == 1
                    && serde_json::from_value::<CalendarEvent>(events[0].clone())
                        .is_ok_and(|saved| saved == *event)
            })
        {
            return Ok(());
        }
        // Older servers acknowledge with a count/object instead of event objects.
        // Read back the saved record rather than retrying a POST that may have succeeded.
        if self.existing()?.iter().any(|saved| saved == event) {
            Ok(())
        } else {
            Err("AW did not confirm the inserted event on read-back".into())
        }
    }
    pub fn delete(&self, id: u64) -> Result<()> {
        let endpoint = self.url(&format!("/events/{id}"));
        match self.agent.delete(&endpoint).call() {
            Ok(_) | Err(ureq::Error::Status(404, _)) => (),
            Err(e) => return Err(http_error(e, "delete calendar event")),
        }
        match self.agent.get(&endpoint).call() {
            Err(ureq::Error::Status(404, _)) => Ok(()),
            Ok(_) => Err("deleted calendar event is still present".into()),
            Err(e) => Err(http_error(e, "verify calendar event deletion")),
        }
    }
}

pub struct Report {
    pub eligible: usize,
    pub inserted: usize,
    pub unchanged: usize,
    pub changed: usize,
    pub deleted: usize,
}
pub fn synchronize(config: &Config, window: Window, dry_run: bool) -> Result<Report> {
    config.validate()?;
    let feed = load_feed(config)?;
    let desired = parse_calendar(&feed, config, window)?;
    if dry_run {
        return Ok(Report {
            eligible: desired.len(),
            inserted: 0,
            unchanged: 0,
            changed: 0,
            deleted: 0,
        });
    }
    let adapter = AwAdapter::new(config)?;
    adapter.ensure_bucket()?;
    let records = adapter.records()?;
    if config.reconcile {
        let plan = plan_reconcile(&desired, &records, window);
        let mut report = Report {
            eligible: desired.len(),
            inserted: 0,
            unchanged: plan.unchanged,
            changed: plan.replaced,
            deleted: 0,
        };
        // Confirm every replacement before removing originals. Interrupted runs are
        // recovered by matching the saved replacement and removing duplicates next time.
        for event in &plan.insert {
            if event.timestamp + chrono::Duration::seconds(event.duration as i64) > Utc::now() {
                return Err("clock moved backwards; reconciliation deferred".into());
            }
            adapter.insert(event)?;
            report.inserted += 1;
        }
        if !plan.delete.is_empty() && Utc::now() < window.end {
            return Err("clock moved backwards; deletion deferred".into());
        }
        for id in plan.delete {
            adapter.delete(id)?;
            report.deleted += 1;
        }
        return Ok(report);
    }
    let existing: Vec<_> = records.into_iter().map(|record| record.event).collect();
    let Plan {
        insert,
        unchanged,
        changed,
    } = plan_import(&desired, &existing);
    let mut report = Report {
        eligible: desired.len(),
        inserted: 0,
        unchanged,
        changed,
        deleted: 0,
    };
    for event in &insert {
        // Even if downloading takes time, the fixed captured window never includes future events.
        if event.timestamp + chrono::Duration::seconds(event.duration as i64) > Utc::now() {
            return Err("clock moved backwards; import deferred".into());
        }
        adapter.insert(event)?;
        report.inserted += 1;
    }
    Ok(report)
}
