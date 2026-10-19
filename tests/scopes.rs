use aw_calendar_sync::{
    config::CalendarSet, parse_calendar, plan_reconcile_with_scope, Config, ImportScope,
    StoredEvent,
};
use chrono::{DateTime, Utc};

fn now() -> DateTime<Utc> {
    "2026-10-19T11:30:00Z".parse().unwrap()
}
fn config(scope: ImportScope) -> Config {
    Config {
        import_scope: scope,
        calendar_file: Some("synthetic.ics".into()),
        default_timezone: "UTC".into(),
        ..Config::default()
    }
}
fn event(id: &str, start: &str, end: &str, extra: &str) -> String {
    format!("BEGIN:VEVENT\r\nUID:{id}\r\nDTSTART:{start}\r\nDTEND:{end}\r\nSUMMARY:Synthetic meeting\r\n{extra}END:VEVENT\r\n")
}
fn feed() -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}END:VCALENDAR\r\n",
        [
            event("past", "20261018T100000Z", "20261018T110000Z", ""),
            event("overnight", "20261018T233000Z", "20261019T120000Z", ""),
            event("done", "20261019T100000Z", "20261019T110000Z", ""),
            event("running", "20261019T110000Z", "20261019T120000Z", ""),
            event("future", "20261019T140000Z", "20261019T150000Z", ""),
            event(
                "across-midnight",
                "20261019T233000Z",
                "20261020T010000Z",
                ""
            ),
            event("tomorrow", "20261020T000000Z", "20261020T010000Z", ""),
            event(
                "oof",
                "20261019T120000Z",
                "20261019T130000Z",
                "X-MICROSOFT-CDO-BUSYSTATUS:OOF\r\n"
            ),
        ]
        .concat()
    )
}

#[test]
fn scopes_include_completed_running_or_whole_local_day_as_requested() {
    for (scope, ids) in [
        (ImportScope::Completed, vec!["past", "done"]),
        (
            ImportScope::Started,
            vec!["past", "overnight", "done", "running"],
        ),
        (
            ImportScope::CurrentDay,
            vec!["done", "running", "future", "across-midnight"],
        ),
    ] {
        let c = config(scope);
        let events = parse_calendar(&feed(), &c, c.window(now()).unwrap()).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|e| e.data.uid.as_str())
                .collect::<Vec<_>>(),
            ids
        );
        if let Some(running) = events.iter().find(|e| e.data.uid == "running") {
            assert_eq!(running.duration, 3600.0); // Full schedule, never elapsed work time.
        }
    }
}

#[test]
fn local_day_handles_both_dst_transitions_and_midnight_exclusively() {
    for (date, hours, start, end) in [
        (
            "2026-03-29T12:00:00Z",
            23,
            "2026-03-28T23:00:00Z",
            "2026-03-29T22:00:00Z",
        ),
        (
            "2026-10-25T12:00:00Z",
            25,
            "2026-10-24T22:00:00Z",
            "2026-10-25T23:00:00Z",
        ),
    ] {
        let c = Config {
            default_timezone: "Europe/Prague".into(),
            ..config(ImportScope::CurrentDay)
        };
        let w = c.window(date.parse().unwrap()).unwrap();
        assert_eq!(w.start, start.parse::<DateTime<Utc>>().unwrap());
        assert_eq!(w.end, end.parse::<DateTime<Utc>>().unwrap());
        assert_eq!((w.end - w.start).num_hours(), hours);
    }
    let c = Config {
        lookback_days: 0,
        ..config(ImportScope::Started)
    };
    let events = parse_calendar(&feed(), &c, c.window(now()).unwrap()).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e.data.uid.as_str())
            .collect::<Vec<_>>(),
        vec!["done", "running"]
    );
}

#[test]
fn reconciliation_handles_running_and_future_records_without_rewriting_frozen_history() {
    for scope in [ImportScope::Started, ImportScope::CurrentDay] {
        let c = config(scope);
        let w = c.window(now()).unwrap();
        let events = parse_calendar(&feed(), &c, w).unwrap();
        let existing: Vec<_> = events
            .iter()
            .enumerate()
            .map(|(i, e)| StoredEvent {
                id: i as u64 + 1,
                event: e.clone(),
            })
            .collect();
        let replay = plan_reconcile_with_scope(&events, &existing, w, scope);
        assert!(replay.insert.is_empty());
        assert!(replay.delete.is_empty());
        let removed = plan_reconcile_with_scope(&[], &existing, w, scope);
        assert_eq!(removed.delete.len(), existing.len());
        let c_default = config(ImportScope::Completed);
        let safe = plan_reconcile_with_scope(
            &[],
            &existing,
            c_default.window(now()).unwrap(),
            ImportScope::Completed,
        );
        for record in &existing {
            if record.event.timestamp + chrono::Duration::seconds(record.event.duration as i64)
                > now()
            {
                assert!(!safe.delete.contains(&record.id));
            }
        }
    }
}

#[test]
fn legacy_default_shared_setting_and_per_calendar_overrides_are_validated() {
    let legacy = CalendarSet::from_json(r#"{"calendar_file":"synthetic.ics"}"#).unwrap();
    assert_eq!(legacy.calendars[0].import_scope, ImportScope::Completed);
    let set=CalendarSet::from_json(r#"{"import_scope":"started","calendars":[{"calendar_id":"one","calendar_file":"one.ics"},{"calendar_id":"two","calendar_file":"two.ics","import_scope":"current_day"}]}"#).unwrap();
    assert_eq!(set.calendars[0].import_scope, ImportScope::Started);
    assert_eq!(set.calendars[1].import_scope, ImportScope::CurrentDay);
    assert!(CalendarSet::from_json(
        r#"{"calendar_file":"synthetic.ics","import_scope":"anything"}"#
    )
    .is_err());
    assert_eq!(CalendarSet::template(false)["import_scope"], "completed");
}

#[test]
fn all_day_and_recurring_events_obey_the_selected_scope() {
    let text="BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:all-day\r\nDTSTART;VALUE=DATE:20261019\r\nDTEND;VALUE=DATE:20261020\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:recurring\r\nDTSTART:20261018T140000Z\r\nDTEND:20261018T150000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    for (scope, count) in [
        (ImportScope::Completed, 1),
        (ImportScope::Started, 2),
        (ImportScope::CurrentDay, 2),
    ] {
        let c = config(scope);
        let events = parse_calendar(text, &c, c.window(now()).unwrap()).unwrap();
        assert_eq!(events.len(), count);
        if scope == ImportScope::CurrentDay {
            assert!(events
                .iter()
                .all(|e| e.timestamp.date_naive() == now().date_naive()));
            assert_eq!(
                events
                    .iter()
                    .find(|e| e.data.uid == "all-day")
                    .unwrap()
                    .duration,
                86400.0
            );
        }
    }
}
