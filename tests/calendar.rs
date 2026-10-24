use aw_watcher_calendars::{parse_calendar, plan_import, Config, Window};
use chrono::{DateTime, Utc};
fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn config() -> Config {
    Config {
        calendar_file: Some("synthetic.ics".into()),
        ..Config::default()
    }
}
fn window() -> Window {
    Window {
        start: utc("2026-10-05T00:00:00Z"),
        end: utc("2026-10-05T12:00:00Z"),
    }
}
fn calendar(events: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Synthetic tests//EN\r\n{events}END:VCALENDAR\r\n")
}
fn event(uid: &str, start: &str, end: &str, extra: &str) -> String {
    format!("BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{start}\r\nDTEND:{end}\r\nSUMMARY:Synthetic meeting\r\n{extra}END:VEVENT\r\n")
}

#[test]
fn only_today_completed_no_future_or_ongoing() {
    let feed = calendar(
        &[
            event("done", "20261005T090000Z", "20261005T100000Z", ""),
            event("ongoing", "20261005T110000Z", "20261005T130000Z", ""),
            event("future", "20261005T140000Z", "20261005T150000Z", ""),
            event("yesterday", "20261004T090000Z", "20261004T100000Z", ""),
            event("boundary", "20261005T110000Z", "20261005T120000Z", ""),
        ]
        .concat(),
    );
    let result = parse_calendar(&feed, &config(), window()).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].data.uid, "done");
    assert_eq!(result[1].data.uid, "boundary");
    assert!(result
        .iter()
        .all(|e| e.timestamp + chrono::Duration::seconds(e.duration as i64) <= window().end));
}
#[test]
fn today_is_local_midnight_not_utc_midnight() {
    let c = Config {
        lookback_days: 0,
        ..config()
    };
    let w = c.window(utc("2026-10-05T12:00:00Z")).unwrap();
    assert_eq!(w.start, utc("2026-10-04T22:00:00Z"));
    let w = c.window(utc("2026-10-05T00:30:00Z")).unwrap();
    let result = parse_calendar(
        &calendar(&event("early", "20261004T223000Z", "20261004T233000Z", "")),
        &config(),
        w,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
}
#[test]
fn recurrence_exclusions_moved_and_cancelled_exceptions() {
    let master = event(
        "weekly",
        "20260928T090000Z",
        "20260928T100000Z",
        "RRULE:FREQ=WEEKLY;COUNT=5\r\nEXDATE:20261012T090000Z\r\n",
    );
    let moved = event(
        "weekly",
        "20261005T103000Z",
        "20261005T113000Z",
        "RECURRENCE-ID:20261005T090000Z\r\nSUMMARY:Moved\r\n",
    );
    let result =
        parse_calendar(&calendar(&(master.clone() + &moved)), &config(), window()).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].timestamp, utc("2026-10-05T10:30:00Z"));
    assert_eq!(result[0].data.occurrence, "2026-10-05T09:00:00+00:00");
    let cancelled = "BEGIN:VEVENT\r\nUID:weekly\r\nRECURRENCE-ID:20261005T090000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n";
    assert!(parse_calendar(
        &calendar(&(master.clone() + cancelled)),
        &config(),
        window()
    )
    .unwrap()
    .is_empty());
    let w = Window {
        start: utc("2026-10-12T00:00:00Z"),
        end: utc("2026-10-12T12:00:00Z"),
    };
    assert!(parse_calendar(&calendar(&master), &config(), w)
        .unwrap()
        .is_empty());
}
#[test]
fn recurrence_expands_across_dst_using_outlook_timezone() {
    let feed = calendar("BEGIN:VEVENT\r\nUID:dst\r\nDTSTART;TZID=Central Europe Standard Time:20261024T090000\r\nDTEND;TZID=Central Europe Standard Time:20261024T100000\r\nRRULE:FREQ=DAILY;COUNT=3\r\nSUMMARY:DST synthetic\r\nEND:VEVENT\r\n");
    let result = parse_calendar(
        &feed,
        &config(),
        Window {
            start: utc("2026-10-24T00:00:00Z"),
            end: utc("2026-10-27T00:00:00Z"),
        },
    )
    .unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].timestamp, utc("2026-10-24T07:00:00Z"));
    assert_eq!(result[1].timestamp, utc("2026-10-25T08:00:00Z"));
    assert_eq!(result[2].timestamp, utc("2026-10-26T08:00:00Z"));
}
#[test]
fn rdate_exdate_without_rrule_and_float_times() {
    let feed = calendar(&event(
        "rdates",
        "20261004T090000",
        "20261004T100000",
        "RDATE:20261005T090000,20261005T110000\r\nEXDATE:20261005T090000\r\n",
    ));
    let result = parse_calendar(&feed, &config(), window()).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].timestamp, utc("2026-10-05T09:00:00Z"));
}
#[test]
fn all_day_span_respects_dst_and_completion() {
    let feed = calendar("BEGIN:VEVENT\r\nUID:day\r\nDTSTART;VALUE=DATE:20261025\r\nDTEND;VALUE=DATE:20261026\r\nSUMMARY:Synthetic all day\r\nEND:VEVENT\r\n");
    let result = parse_calendar(
        &feed,
        &config(),
        Window {
            start: utc("2026-10-24T00:00:00Z"),
            end: utc("2026-10-26T00:00:00Z"),
        },
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].data.all_day, Some(true));
    assert_eq!(result[0].duration, 25.0 * 3600.0);
    let w = config().window(utc("2026-10-25T15:00:00Z")).unwrap();
    assert!(parse_calendar(&feed, &config(), w).unwrap().is_empty());
}
#[test]
fn never_reinsert_modify_or_delete_retained_events() {
    let original = parse_calendar(
        &calendar(&event("one", "20261005T090000Z", "20261005T100000Z", "")),
        &config(),
        window(),
    )
    .unwrap();
    assert_eq!(plan_import(&original, &[]).insert.len(), 1);
    let plan = plan_import(&original, &original);
    assert!(plan.insert.is_empty());
    assert_eq!(plan.unchanged, 1);
    let mut edited = original.clone();
    edited[0].data.title = "Edited later".into();
    edited[0].timestamp = utc("2026-10-05T10:00:00Z");
    let plan = plan_import(&edited, &original);
    assert!(plan.insert.is_empty());
    assert_eq!(plan.changed, 1);
    assert!(plan_import(&[], &original).insert.is_empty());
}
#[test]
fn moved_occurrence_keeps_identity_even_across_day_boundary() {
    let feed = calendar(&event(
        "moved",
        "20261005T090000Z",
        "20261005T100000Z",
        "RECURRENCE-ID:20261004T090000Z\r\n",
    ));
    let result = parse_calendar(&feed, &config(), window()).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].data.occurrence, "2026-10-04T09:00:00+00:00");
}
#[test]
fn malformed_and_unsupported_feeds_fail_before_writes() {
    for feed in ["<html>Login</html>".into(), "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n".into(),
        calendar(&event("bad","20261005T090000Z","20261005T080000Z","")),
        calendar(&event("bad","20261005T090000Z","20261005T100000Z","RRULE:FREQ=INVALID\r\n")),
        calendar(&event("bad","20261005T090000Z","20261005T100000Z","RECURRENCE-ID;RANGE=THISANDFUTURE:20261005T090000Z\r\n")),
        calendar("BEGIN:VEVENT\r\nUID:tz\r\nDTSTART;TZID=Unknown Zone:20261005T090000\r\nDTEND;TZID=Unknown Zone:20261005T100000\r\nEND:VEVENT\r\n") ] {
        assert!(parse_calendar(&feed,&config(),window()).is_err());
    }
}
#[test]
fn highest_sequence_wins_and_cancelled_master_suppresses_exceptions() {
    let feed = calendar(
        &(event(
            "one",
            "20261005T090000Z",
            "20261005T100000Z",
            "SEQUENCE:2\r\n",
        ) + &event(
            "one",
            "20261005T140000Z",
            "20261005T150000Z",
            "SEQUENCE:1\r\n",
        )),
    );
    assert_eq!(parse_calendar(&feed, &config(), window()).unwrap().len(), 1);
    let feed = calendar(
        &("BEGIN:VEVENT\r\nUID:one\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n".to_string()
            + &event(
                "one",
                "20261005T090000Z",
                "20261005T100000Z",
                "RECURRENCE-ID:20261005T090000Z\r\n",
            )),
    );
    assert!(parse_calendar(&feed, &config(), window())
        .unwrap()
        .is_empty());
}
#[test]
fn duration_and_folded_text_and_attendee_opt_in() {
    let feed = calendar("BEGIN:VEVENT\r\nUID:duration\r\nDTSTART:20261005T090000Z\r\nDURATION:PT1H30M\r\nSUMMARY:Hello\\, world\\nsecond\r\n line\r\nATTENDEE:mailto:synthetic@example.invalid\r\nEND:VEVENT\r\n");
    let result = parse_calendar(&feed, &config(), window()).unwrap();
    assert_eq!(result[0].duration, 5400.0);
    assert_eq!(result[0].data.title, "Hello, world secondline");
    assert!(result[0].data.attendees.is_empty());
    let result = parse_calendar(
        &feed,
        &Config {
            include_attendees: true,
            ..config()
        },
        window(),
    )
    .unwrap();
    assert_eq!(result[0].data.attendees.len(), 1);
}
#[test]
fn source_and_server_validation_and_seven_day_option() {
    config().validate().unwrap();
    for server in [
        "https://remote.example.invalid",
        "http://127.0.0.1.example.invalid:5600",
        "http://localhost:5600/path",
    ] {
        assert!(Config {
            server_url: server.into(),
            ..config()
        }
        .validate()
        .is_err());
    }
    assert!(Config {
        lookback_days: 8,
        ..config()
    }
    .validate()
    .is_err());
    let c = Config {
        lookback_days: 7,
        ..config()
    };
    assert_eq!(
        c.window(window().end).unwrap().start,
        utc("2026-09-28T12:00:00Z")
    );
    assert_eq!(Config::default().poll_seconds, 3600);
    assert_eq!(Config::default().lookback_days, 7);
}

#[test]
fn rolling_window_catches_overnight_missed_meetings_and_retains_older_aw_records() {
    let c = config();
    let w = c.window(utc("2026-10-06T08:00:00Z")).unwrap();
    let feed = calendar(
        &[
            event("late", "20261005T210000Z", "20261005T220000Z", ""),
            event("overnight", "20261005T230000Z", "20261006T010000Z", ""),
            event("too-old", "20260928T090000Z", "20260928T100000Z", ""),
            event("not-done", "20261006T070000Z", "20261006T090000Z", ""),
        ]
        .concat(),
    );
    let desired = parse_calendar(&feed, &c, w).unwrap();
    assert_eq!(desired.len(), 2);
    assert_eq!(plan_import(&desired, &[]).insert.len(), 2);
    assert!(plan_import(&desired, &desired).insert.is_empty());
    let next = c.window(utc("2026-10-13T08:00:00Z")).unwrap();
    assert!(parse_calendar(&feed, &c, next)
        .unwrap()
        .iter()
        .all(|e| e.data.uid != "late"));
    assert!(plan_import(&[], &desired).insert.is_empty());
}
