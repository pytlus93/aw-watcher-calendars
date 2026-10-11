//! Multiple sources with shared defaults; legacy single-source JSON remains supported.
use crate::{Config, PayloadMode, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarSource {
    pub calendar_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_file: Option<String>,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded_busy_statuses: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconcile: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_attendees: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_mode: Option<PayloadMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_summary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_all_day: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_status: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookback_days: Option<u32>,
}
fn enabled() -> bool {
    true
}
pub struct CalendarSet {
    pub poll_seconds: u64,
    pub server_url: String,
    pub calendars: Vec<Config>,
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    // Never display serde's message: invalid field values may contain private URLs.
    serde_json::from_value(value)
        .map_err(|_| "invalid calendar configuration fields or types".into())
}
impl CalendarSet {
    pub fn from_json(text: &str) -> Result<Self> {
        let mut value: Value = serde_json::from_str(text).map_err(|e| {
            format!(
                "invalid config JSON at line {}, column {}",
                e.line(),
                e.column()
            )
        })?;
        let object = value
            .as_object_mut()
            .ok_or("config must be a JSON object")?;
        let Some(sources) = object.remove("calendars") else {
            let config: Config = decode(value)?;
            config.validate()?;
            return Ok(Self {
                poll_seconds: config.poll_seconds,
                server_url: config.server_url.clone(),
                calendars: vec![config],
            });
        };
        if ["calendar_id", "calendar_url", "calendar_file"]
            .iter()
            .any(|key| object.contains_key(*key))
        {
            return Err("put calendar_id, calendar_url and calendar_file inside calendars entries, not at the top level".into());
        }
        let sources: Vec<CalendarSource> = decode(sources)?;
        if sources.is_empty() || sources.len() > 32 {
            return Err("calendars must contain 1..32 entries".into());
        }
        let shared: Config = decode(value)?;
        let mut ids = HashSet::new();
        let mut calendars = Vec::new();
        for source in sources {
            // Validate identity even on disabled entries; these IDs own retained buckets.
            let mut config = shared.clone();
            config.calendar_id = source.calendar_id;
            config.calendar_url = source.calendar_url;
            config.calendar_file = source.calendar_file;
            if let Some(statuses) = source.excluded_busy_statuses {
                config.excluded_busy_statuses = statuses;
            }
            if let Some(reconcile) = source.reconcile {
                config.reconcile = reconcile;
            }
            if let Some(timezone) = source.default_timezone {
                config.default_timezone = timezone;
            }
            if let Some(attendees) = source.include_attendees {
                config.include_attendees = attendees;
            }
            if let Some(mode) = source.payload_mode {
                config.payload_mode = mode;
            }
            if let Some(enabled) = source.include_summary {
                config.include_summary = enabled;
            }
            if let Some(enabled) = source.include_all_day {
                config.include_all_day = enabled;
            }
            if let Some(enabled) = source.include_status {
                config.include_status = enabled;
            }
            if let Some(days) = source.lookback_days {
                config.lookback_days = days;
            }
            if !source.enabled {
                // An inactive template may have no URL yet. Still validate all shared settings.
                config.calendar_url = None;
                config.calendar_file = Some("disabled-source.ics".into());
            }
            config.validate()?;
            if !ids.insert(config.calendar_id.to_ascii_lowercase()) {
                return Err("duplicate calendar_id in calendars".into());
            }
            if source.enabled {
                calendars.push(config);
            }
        }
        Ok(Self {
            poll_seconds: shared.poll_seconds,
            server_url: shared.server_url,
            calendars,
        })
    }
    pub fn template(testing: bool) -> Value {
        let mut settings = Config {
            payload_mode: PayloadMode::Strict,
            ..Config::default()
        };
        if testing {
            settings.server_url = "http://127.0.0.1:5666".into();
        }
        let mut value = serde_json::to_value(settings).unwrap();
        let object = value.as_object_mut().unwrap();
        for key in ["calendar_id", "calendar_url", "calendar_file"] {
            object.remove(key);
        }
        object.insert("calendars".into(),serde_json::json!([
            {"calendar_id":"outlook-work", "calendar_url":"https://outlook.office365.com/REPLACE_WITH_YOUR_PRIVATE_ICS_LINK/calendar.ics", "enabled":true}
        ]));
        value
    }
}
