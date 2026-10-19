//! Host-independent calendar parsing and append-only import planning.
pub mod adapter;
pub mod config;
use chrono::{DateTime, Days, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use ical::{parser::ical::component::IcalEvent, property::Property};
use rrule::{RRule, RRuleSet, Tz as RuleTz};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Cursor,
};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadMode {
    Strict,
    Extended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportScope {
    Completed,
    Started,
    CurrentDay,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub calendar_url: Option<String>,
    pub calendar_file: Option<String>,
    pub calendar_id: String,
    pub poll_seconds: u64,
    pub server_url: String,
    pub default_timezone: String,
    pub timezone_aliases: BTreeMap<String, String>,
    pub include_attendees: bool,
    pub payload_mode: PayloadMode,
    pub include_summary: bool,
    pub include_all_day: bool,
    pub include_status: bool,
    pub excluded_busy_statuses: Vec<String>,
    pub reconcile: bool,
    /// 0 = current local calendar day, 1..=7 = rolling days.
    pub lookback_days: u32,
    pub import_scope: ImportScope,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            calendar_url: None,
            calendar_file: None,
            calendar_id: "outlook-work".into(),
            poll_seconds: 3600,
            server_url: "http://127.0.0.1:5600".into(),
            default_timezone: "Europe/Prague".into(),
            timezone_aliases: [
                ("Central Europe Standard Time", "Europe/Prague"),
                ("Central European Standard Time", "Europe/Warsaw"),
                ("W. Europe Standard Time", "Europe/Berlin"),
                ("Romance Standard Time", "Europe/Paris"),
                ("GMT Standard Time", "Europe/London"),
                ("China Standard Time", "Asia/Shanghai"),
                ("Eastern Standard Time", "America/New_York"),
                ("GTB Standard Time", "Europe/Bucharest"),
                ("Sri Lanka Standard Time", "Asia/Colombo"),
                ("UTC", "UTC"),
            ]
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect(),
            include_attendees: false,
            // Preserve the pre-mode payload for existing configurations.
            payload_mode: PayloadMode::Extended,
            include_summary: true,
            include_all_day: true,
            include_status: true,
            excluded_busy_statuses: vec!["OOF".into()],
            reconcile: true,
            lookback_days: 7,
            import_scope: ImportScope::Completed,
        }
    }
}
impl Config {
    pub fn timezone(&self, name: &str) -> Result<Tz> {
        // Retain built-in aliases when an older config contains only the original set.
        let defaults = Self::default();
        self.timezone_aliases
            .get(name)
            .or_else(|| defaults.timezone_aliases.get(name))
            .map(String::as_str)
            .unwrap_or(name)
            .parse()
            .map_err(|_| {
                let diagnostic = if name.len() <= 100
                    && !name.contains("://")
                    && !name.chars().any(char::is_control)
                {
                    format!("{name:?}")
                } else {
                    "custom timezone identifier (redacted)".into()
                };
                format!(
                    "unrecognized timezone {diagnostic}; add an IANA mapping to timezone_aliases"
                )
            })
    }
    pub fn validate(&self) -> Result<()> {
        if self
            .excluded_busy_statuses
            .iter()
            .any(|s| s.is_empty() || s.len() > 32 || !s.bytes().all(|c| c.is_ascii_alphabetic()))
        {
            return Err(
                "excluded_busy_statuses must contain status names such as OOF, FREE or BUSY".into(),
            );
        }
        if self.calendar_url.is_some() == self.calendar_file.is_some() {
            return Err("configure exactly one of calendar_url or calendar_file".into());
        }
        if let Some(raw) = &self.calendar_url {
            let u = url::Url::parse(raw).map_err(|_| "invalid calendar URL")?;
            if u.scheme() != "https"
                || u.host_str().is_none()
                || !u.username().is_empty()
                || u.password().is_some()
            {
                return Err(
                    "calendar_url must be an HTTPS URL without embedded credentials".into(),
                );
            }
        }
        if self.calendar_id.is_empty()
            || self.calendar_id.len() > 80
            || !self
                .calendar_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(
                "calendar_id must contain 1..80 ASCII letters, numbers, hyphens or underscores"
                    .into(),
            );
        }
        if !(60..=86400).contains(&self.poll_seconds) || self.lookback_days > 7 {
            return Err("poll_seconds must be 60..86400 and lookback_days 0..7".into());
        }
        let u = url::Url::parse(&self.server_url).map_err(|_| "invalid server URL")?;
        if u.scheme() != "http"
            || !matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
            || u.path() != "/"
            || u.query().is_some()
            || u.fragment().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
        {
            return Err(
                "server_url must be a loopback HTTP origin, e.g. http://127.0.0.1:5600".into(),
            );
        }
        self.timezone(&self.default_timezone)?;
        for alias in self.timezone_aliases.values() {
            alias.parse::<Tz>().map_err(|_| "invalid timezone alias")?;
        }
        Ok(())
    }
    pub fn bucket_id(&self) -> String {
        format!("aw-calendar-sync_{}", self.calendar_id)
    }
    pub fn window(&self, now: DateTime<Utc>) -> Result<Window> {
        let start = if self.lookback_days == 0 || self.import_scope == ImportScope::CurrentDay {
            let tz = self.timezone(&self.default_timezone)?;
            tz.from_local_datetime(
                &now.with_timezone(&tz)
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            )
            .earliest()
            .ok_or("local midnight does not exist in configured timezone")?
            .with_timezone(&Utc)
        } else {
            now - Duration::days(self.lookback_days as i64)
        };
        let end = if self.import_scope == ImportScope::CurrentDay {
            let tz = self.timezone(&self.default_timezone)?;
            let tomorrow = now
                .with_timezone(&tz)
                .date_naive()
                .succ_opt()
                .ok_or("invalid next local day")?;
            tz.from_local_datetime(&tomorrow.and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .ok_or("next local midnight does not exist")?
                .with_timezone(&Utc)
        } else {
            now
        };
        Ok(Window { start, end })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarData {
    pub schema_version: u32,
    pub calendar_id: String,
    pub uid: String,
    /// Original occurrence start, unchanged when an exception moves a meeting.
    pub occurrence: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_day: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub timestamp: DateTime<Utc>,
    pub duration: f64,
    pub data: CalendarData,
}
impl CalendarEvent {
    pub fn key(&self) -> (String, String) {
        (self.data.uid.clone(), self.data.occurrence.clone())
    }
}
pub struct Plan {
    pub insert: Vec<CalendarEvent>,
    pub unchanged: usize,
    pub changed: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredEvent {
    pub id: u64,
    #[serde(flatten)]
    pub event: CalendarEvent,
}
pub struct ReconcilePlan {
    pub insert: Vec<CalendarEvent>,
    pub delete: Vec<u64>,
    pub unchanged: usize,
    pub replaced: usize,
}
fn within_scope(event: &CalendarEvent, window: Window, scope: ImportScope) -> bool {
    event.timestamp >= window.start
        && if scope == ImportScope::CurrentDay {
            event.timestamp < window.end
        } else {
            event.timestamp <= window.end
        }
        && event.duration.is_finite()
        && event.duration >= 0.0
        && event.duration <= 366.0 * 86400.0
        && (scope != ImportScope::Completed
            || event.timestamp + Duration::seconds(event.duration as i64) <= window.end)
}
/// Scope destructive actions to completed records within the active window.
pub fn plan_reconcile(
    desired: &[CalendarEvent],
    existing: &[StoredEvent],
    window: Window,
) -> ReconcilePlan {
    plan_reconcile_with_scope(desired, existing, window, ImportScope::Completed)
}

pub fn plan_reconcile_with_scope(
    desired: &[CalendarEvent],
    existing: &[StoredEvent],
    window: Window,
    scope: ImportScope,
) -> ReconcilePlan {
    let mut plan = ReconcilePlan {
        insert: Vec::new(),
        delete: Vec::new(),
        unchanged: 0,
        replaced: 0,
    };
    let mut groups: HashMap<_, Vec<&StoredEvent>> = HashMap::new();
    for old in existing {
        groups.entry(old.event.key()).or_default().push(old);
    }
    let mut seen = HashSet::new();
    for event in desired {
        if !seen.insert(event.key()) || !within_scope(event, window, scope) {
            continue;
        }
        let olds = groups.remove(&event.key()).unwrap_or_default();
        // A moved historical occurrence must not rewrite frozen older history.
        if olds
            .iter()
            .any(|old| !within_scope(&old.event, window, scope))
        {
            plan.unchanged += 1;
            continue;
        }
        if let Some(retain) = olds.iter().find(|old| old.event == *event) {
            plan.unchanged += 1;
            plan.delete.extend(
                olds.iter()
                    .filter(|old| old.id != retain.id)
                    .map(|old| old.id),
            );
        } else {
            plan.insert.push(event.clone());
            if !olds.is_empty() {
                plan.replaced += 1;
            }
            plan.delete.extend(olds.iter().map(|old| old.id));
        }
    }
    for old in groups.values().flatten() {
        if within_scope(&old.event, window, scope) {
            plan.delete.push(old.id);
        }
    }
    plan.delete.sort_unstable();
    plan.delete.dedup();
    plan
}
/// Existing records are immutable. No operation in this plan deletes or updates.
pub fn plan_import(desired: &[CalendarEvent], existing: &[CalendarEvent]) -> Plan {
    let known: HashMap<_, _> = existing.iter().map(|e| (e.key(), e)).collect();
    let mut seen = HashSet::new();
    let mut plan = Plan {
        insert: Vec::new(),
        unchanged: 0,
        changed: 0,
    };
    for e in desired {
        if !seen.insert(e.key()) {
            continue;
        }
        match known.get(&e.key()) {
            None => plan.insert.push(e.clone()),
            Some(old) if *old == e => plan.unchanged += 1,
            Some(_) => plan.changed += 1,
        }
    }
    plan
}

fn props<'a>(event: &'a IcalEvent, name: &'a str) -> impl Iterator<Item = &'a Property> {
    event.properties.iter().filter(move |p| p.name == name)
}
fn prop<'a>(event: &'a IcalEvent, name: &'a str) -> Option<&'a Property> {
    props(event, name).next()
}
fn value<'a>(event: &'a IcalEvent, name: &'a str) -> Option<&'a str> {
    prop(event, name)?.value.as_deref()
}

fn excluded_busy_status(event: &IcalEvent, master: Option<&IcalEvent>, config: &Config) -> bool {
    // Actual show-as status wins over intended status. Exceptions inherit absent fields.
    let actual = value(event, "X-MICROSOFT-CDO-BUSYSTATUS")
        .or_else(|| master.and_then(|m| value(m, "X-MICROSOFT-CDO-BUSYSTATUS")));
    let intended = value(event, "X-MICROSOFT-CDO-INTENDEDSTATUS")
        .or_else(|| master.and_then(|m| value(m, "X-MICROSOFT-CDO-INTENDEDSTATUS")));
    actual.or(intended).is_some_and(|s| {
        config
            .excluded_busy_statuses
            .iter()
            .any(|excluded| s.trim().eq_ignore_ascii_case(excluded))
    })
}
fn param<'a>(p: &'a Property, name: &str) -> Option<&'a str> {
    p.params
        .as_ref()?
        .iter()
        .find(|(k, _)| k == name)?
        .1
        .first()
        .map(String::as_str)
}
fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(c) => out.push(c),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
#[derive(Clone)]
struct Stamp {
    dt: DateTime<Tz>,
    all_day: bool,
}
fn stamp(p: &Property, text: &str, config: &Config) -> Result<Stamp> {
    let all_day = param(p, "VALUE") == Some("DATE") || text.len() == 8;
    let tz = if text.ends_with('Z') {
        chrono_tz::UTC
    } else {
        config.timezone(param(p, "TZID").unwrap_or(&config.default_timezone))?
    };
    let naive = if all_day {
        NaiveDate::parse_from_str(text, "%Y%m%d")
            .map_err(|_| "invalid calendar date")?
            .and_hms_opt(0, 0, 0)
            .unwrap()
    } else {
        NaiveDateTime::parse_from_str(text.trim_end_matches('Z'), "%Y%m%dT%H%M%S")
            .map_err(|_| "invalid calendar datetime")?
    };
    // RFC 5545: first occurrence when local time repeats. Nonexistent times fail safely.
    let dt = tz
        .from_local_datetime(&naive)
        .earliest()
        .ok_or("calendar time falls in a DST gap")?;
    Ok(Stamp { dt, all_day })
}
fn required_stamp(event: &IcalEvent, name: &str, config: &Config) -> Result<Stamp> {
    let p = prop(event, name).ok_or("event missing required date property")?;
    stamp(p, p.value.as_deref().ok_or("empty date property")?, config)
}

fn duration_value(text: &str) -> Result<Duration> {
    // Positive RFC 5545 durations, including PnW and PnDTnHnMnS.
    let text = text.strip_prefix('+').unwrap_or(text);
    let tail = text.strip_prefix('P').ok_or("invalid event duration")?;
    let mut num = String::new();
    let mut seconds = 0i64;
    let mut in_time = false;
    let mut seen = HashSet::new();
    let mut units = 0;
    let mut last_rank = 0;
    for c in tail.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        if c == 'T' && num.is_empty() && !in_time {
            in_time = true;
            continue;
        }
        let (factor, rank) = match (c, in_time) {
            ('W', false) => (604800, 1),
            ('D', false) => (86400, 2),
            ('H', true) => (3600, 3),
            ('M', true) => (60, 4),
            ('S', true) => (1, 5),
            _ => return Err("unsupported event duration".into()),
        };
        if !seen.insert(c) || rank <= last_rank || (seen.contains(&'W') && seen.len() > 1) {
            return Err("invalid duration units".into());
        }
        last_rank = rank;
        units += 1;
        let n: i64 = num.parse().map_err(|_| "invalid event duration number")?;
        num.clear();
        seconds = seconds
            .checked_add(n.checked_mul(factor).ok_or("duration overflow")?)
            .ok_or("duration overflow")?;
    }
    if !num.is_empty() || units == 0 || tail.ends_with('T') || seconds > 366 * 86400 {
        return Err("invalid or excessive event duration".into());
    }
    Ok(Duration::seconds(seconds))
}

fn event_span(
    event: &IcalEvent,
    start: &Stamp,
    config: &Config,
) -> Result<(Duration, Option<u64>)> {
    if prop(event, "DTEND").is_some() && prop(event, "DURATION").is_some() {
        return Err("event has both DTEND and DURATION".into());
    }
    if let Some(p) = prop(event, "DTEND") {
        let end = stamp(p, p.value.as_deref().ok_or("empty DTEND")?, config)?;
        if end.all_day != start.all_day {
            return Err("DTSTART and DTEND types differ".into());
        }
        let span = end.dt.signed_duration_since(start.dt);
        if span < Duration::zero() || span > Duration::days(366) {
            return Err("invalid or excessive event span".into());
        }
        let days = if start.all_day {
            Some((end.dt.date_naive() - start.dt.date_naive()).num_days() as u64)
        } else {
            None
        };
        Ok((span, days))
    } else if let Some(d) = value(event, "DURATION") {
        let span = duration_value(d)?;
        if start.all_day && span.num_seconds() % 86400 != 0 {
            return Err("all-day duration must be whole days".into());
        }
        Ok((
            span,
            if start.all_day {
                Some(span.num_days() as u64)
            } else {
                None
            },
        ))
    } else if start.all_day {
        Ok((Duration::days(1), Some(1)))
    } else {
        Ok((Duration::zero(), None))
    }
}

fn make_event(
    event: &IcalEvent,
    start: &Stamp,
    occurrence: DateTime<Utc>,
    span: Duration,
    days: Option<u64>,
    config: &Config,
    window: Window,
) -> Result<Option<CalendarEvent>> {
    let end = if let Some(days) = days {
        start
            .dt
            .checked_add_days(Days::new(days))
            .ok_or("invalid all-day end")?
    } else {
        start
            .dt
            .checked_add_signed(span)
            .ok_or("invalid event end")?
    };
    let utc = start.dt.with_timezone(&Utc);
    // Scope controls start/end eligibility; always retain the full scheduled interval.
    if utc < window.start
        || (if config.import_scope == ImportScope::CurrentDay {
            utc >= window.end
        } else {
            utc > window.end
        })
        || (config.import_scope == ImportScope::Completed && end.with_timezone(&Utc) > window.end)
        || value(event, "STATUS") == Some("CANCELLED")
    {
        return Ok(None);
    }
    Ok(Some(CalendarEvent {
        timestamp: utc,
        duration: end.signed_duration_since(start.dt).num_seconds() as f64,
        data: CalendarData {
            schema_version: 1,
            calendar_id: config.calendar_id.clone(),
            uid: value(event, "UID").ok_or("event missing UID")?.to_string(),
            occurrence: occurrence.to_rfc3339(),
            title: if config.include_summary {
                let summary = unescape(value(event, "SUMMARY").unwrap_or(""));
                let concise = summary.split_whitespace().collect::<Vec<_>>().join(" ");
                if concise.is_empty() {
                    "Meeting (title unavailable)".into()
                } else {
                    concise.chars().take(200).collect()
                }
            } else {
                "Meeting (title unavailable)".into()
            },
            all_day: (config.payload_mode == PayloadMode::Extended && config.include_all_day)
                .then_some(start.all_day),
            status: (config.payload_mode == PayloadMode::Extended && config.include_status)
                .then(|| value(event, "STATUS").unwrap_or("CONFIRMED").into()),
            attendees: if config.payload_mode == PayloadMode::Extended && config.include_attendees {
                props(event, "ATTENDEE")
                    .filter_map(|p| p.value.clone())
                    .collect()
            } else {
                Vec::new()
            },
        },
    }))
}

/// Parse the entire feed before any writes. Unsupported/malformed calendars fail closed.
pub fn parse_calendar(text: &str, config: &Config, window: Window) -> Result<Vec<CalendarEvent>> {
    if !text.trim_end().ends_with("END:VCALENDAR") {
        return Err("calendar is incomplete or not an ICS feed".into());
    }
    let mut parser = ical::IcalParser::new(Cursor::new(text.as_bytes()));
    let calendar = parser
        .next()
        .ok_or("empty calendar")?
        .map_err(|_| "invalid ICS structure")?;
    if parser.next().is_some() {
        return Err("expected one VCALENDAR".into());
    }
    if !calendar
        .properties
        .iter()
        .any(|p| p.name == "VERSION" && p.value.as_deref() == Some("2.0"))
    {
        return Err("expected iCalendar VERSION:2.0".into());
    }
    // Prefer the latest revision of a UID/RECURRENCE-ID, without logging private values.
    let mut revisions: BTreeMap<(String, String), IcalEvent> = BTreeMap::new();
    for event in calendar.events {
        let uid = value(&event, "UID")
            .filter(|s| !s.is_empty())
            .ok_or("event missing UID")?
            .to_string();
        if props(&event, "RECURRENCE-ID").any(|p| param(p, "RANGE").is_some()) {
            return Err("RANGE recurrence exceptions are not supported".into());
        }
        if props(&event, "EXRULE").next().is_some() {
            return Err("EXRULE is not supported".into());
        }
        let occurrence = match prop(&event, "RECURRENCE-ID") {
            Some(p) => stamp(p, p.value.as_deref().ok_or("empty recurrence id")?, config)?
                .dt
                .with_timezone(&Utc)
                .to_rfc3339(),
            None => String::new(),
        };
        let key = (uid, occurrence);
        let sequence = |e: &IcalEvent| -> Result<i64> {
            value(e, "SEQUENCE")
                .unwrap_or("0")
                .parse()
                .map_err(|_| "invalid event sequence".into())
        };
        let seq = sequence(&event)?;
        let replace = if let Some(old) = revisions.get(&key) {
            (seq, value(&event, "DTSTAMP").unwrap_or(""))
                >= (sequence(old)?, value(old, "DTSTAMP").unwrap_or(""))
        } else {
            true
        };
        if replace {
            revisions.insert(key, event);
        }
    }
    let mut output = Vec::new();
    for ((uid, rid), event) in &revisions {
        if !rid.is_empty()
            || value(event, "STATUS") == Some("CANCELLED")
            || excluded_busy_status(event, None, config)
        {
            continue;
        }
        let start = required_stamp(event, "DTSTART", config)?;
        let (span, days) = event_span(event, &start, config)?;
        let mut dates = vec![start.dt];
        if prop(event, "RRULE").is_some() || prop(event, "RDATE").is_some() {
            let rstart = start.dt.with_timezone(&RuleTz::Tz(start.dt.timezone()));
            let mut rules = RRuleSet::new(rstart);
            // Explicit DTSTART is part of the recurrence set, including rules which don't match it.
            rules = rules.rdate(rstart);
            for p in props(event, "RRULE") {
                let rule: RRule<rrule::Unvalidated> = p
                    .value
                    .as_deref()
                    .ok_or("empty RRULE")?
                    .parse()
                    .map_err(|_| "invalid RRULE")?;
                rules = rules.rrule(
                    rule.validate(rstart)
                        .map_err(|_| "invalid recurrence rule")?,
                );
            }
            for name in ["RDATE", "EXDATE"] {
                for p in props(event, name) {
                    if param(p, "VALUE") == Some("PERIOD") {
                        return Err("RDATE periods are not supported".into());
                    }
                    for raw in p
                        .value
                        .as_deref()
                        .ok_or("empty recurrence date")?
                        .split(',')
                    {
                        let dt = stamp(p, raw, config)?.dt;
                        let dt = dt.with_timezone(&RuleTz::Tz(dt.timezone()));
                        rules = if name == "RDATE" {
                            rules.rdate(dt)
                        } else {
                            rules.exdate(dt)
                        };
                    }
                }
            }
            let result = rules
                .after((window.start - Duration::seconds(1)).with_timezone(&RuleTz::UTC))
                .before(window.end.with_timezone(&RuleTz::UTC))
                .all(10000);
            if result.limited {
                return Err("recurrence expansion exceeded safety limit".into());
            }
            dates = result
                .dates
                .into_iter()
                .map(|d| d.with_timezone(&start.dt.timezone()))
                .collect();
        } else if prop(event, "EXDATE").is_some() {
            return Err("EXDATE without a recurrence set".into());
        }
        for date in dates {
            let occurrence = date.with_timezone(&Utc);
            if revisions.contains_key(&(uid.clone(), occurrence.to_rfc3339())) {
                continue;
            }
            let actual = Stamp {
                dt: date,
                all_day: start.all_day,
            };
            if let Some(e) = make_event(event, &actual, occurrence, span, days, config, window)? {
                output.push(e);
            }
        }
    }
    // Exceptions are processed independently, including moves into the current window.
    for ((uid, rid), event) in &revisions {
        let master = revisions.get(&(uid.clone(), String::new()));
        if rid.is_empty()
            || excluded_busy_status(event, master, config)
            || value(event, "STATUS") == Some("CANCELLED")
            || revisions
                .get(&(uid.clone(), String::new()))
                .is_some_and(|e| value(e, "STATUS") == Some("CANCELLED"))
        {
            continue;
        }
        let start = required_stamp(event, "DTSTART", config)?;
        let (span, days) = event_span(event, &start, config)?;
        let occurrence = DateTime::parse_from_rfc3339(rid)
            .map_err(|_| "invalid occurrence identity")?
            .with_timezone(&Utc);
        if let Some(e) = make_event(event, &start, occurrence, span, days, config, window)? {
            output.push(e);
        }
    }
    if output.len() > 20000 {
        return Err("calendar event limit exceeded".into());
    }
    output.sort_by_key(|e| (e.timestamp, e.key()));
    output.dedup_by(|a, b| a.key() == b.key());
    Ok(output)
}
