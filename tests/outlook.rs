use aw_calendar_sync::{parse_calendar, Config};

fn filtered_events(events: &str) -> Vec<aw_calendar_sync::CalendarEvent> {
    let c = Config::default();
    let feed = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{events}END:VCALENDAR\r\n");
    parse_calendar(
        &feed,
        &c,
        c.window("2026-10-05T12:00:00Z".parse().unwrap()).unwrap(),
    )
    .unwrap()
}
fn busy_event(uid: &str, fields: &str) -> String {
    format!("BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:20261005T090000Z\r\nDTEND:20261005T100000Z\r\nSUMMARY:Synthetic\r\n{fields}END:VEVENT\r\n")
}
#[test]
fn actual_oof_is_excluded_even_when_intended_busy() {
    let fields = "X-MICROSOFT-CDO-BUSYSTATUS:OOF\r\nX-MICROSOFT-CDO-INTENDEDSTATUS:BUSY\r\n";
    assert!(filtered_events(&busy_event("absence", fields)).is_empty());
    assert!(
        filtered_events(&busy_event("absence", "X-MICROSOFT-CDO-BUSYSTATUS:oof\r\n")).is_empty()
    );
    assert!(filtered_events(&busy_event(
        "absence",
        "X-MICROSOFT-CDO-INTENDEDSTATUS:OOF\r\n"
    ))
    .is_empty());
    assert_eq!(
        filtered_events(&busy_event(
            "work",
            "X-MICROSOFT-CDO-BUSYSTATUS:BUSY\r\nX-MICROSOFT-CDO-INTENDEDSTATUS:OOF\r\n"
        ))
        .len(),
        1
    );
    assert_eq!(filtered_events(&busy_event("unspecified", "")).len(), 1);
}

#[test]
fn busy_status_filter_is_configurable_and_defaults_for_old_configs() {
    let mut c = Config::default();
    let feed = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}END:VCALENDAR\r\n",
        busy_event("absence", "X-MICROSOFT-CDO-BUSYSTATUS:OOF\r\n")
    );
    let w = c.window("2026-10-05T12:00:00Z".parse().unwrap()).unwrap();
    assert!(parse_calendar(&feed, &c, w).unwrap().is_empty());
    c.excluded_busy_statuses.clear();
    assert_eq!(parse_calendar(&feed, &c, w).unwrap().len(), 1);
    c.excluded_busy_statuses = vec!["FREE".into()];
    assert_eq!(parse_calendar(&feed, &c, w).unwrap().len(), 1);
    let free = feed.replace("BUSYSTATUS:OOF", "BUSYSTATUS:FREE");
    assert!(parse_calendar(&free, &c, w).unwrap().is_empty());
    let old: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(old.excluded_busy_statuses, vec!["OOF"]);
}
#[test]
fn recurring_absences_and_exception_status_are_filtered() {
    let master = busy_event(
        "series",
        "RRULE:FREQ=DAILY;COUNT=3\r\nX-MICROSOFT-CDO-BUSYSTATUS:OOF\r\n",
    );
    assert!(filtered_events(&master).is_empty());
    let exception = busy_event("series", "RECURRENCE-ID:20261005T090000Z\r\n");
    assert!(filtered_events(&(master.clone() + &exception)).is_empty());
    let busy = busy_event(
        "series",
        "RECURRENCE-ID:20261005T090000Z\r\nX-MICROSOFT-CDO-BUSYSTATUS:BUSY\r\n",
    );
    assert_eq!(filtered_events(&(master + &busy)).len(), 1);
    let master = busy_event(
        "work",
        "RRULE:FREQ=DAILY;COUNT=3\r\nX-MICROSOFT-CDO-BUSYSTATUS:BUSY\r\n",
    );
    let absence = busy_event(
        "work",
        "RECURRENCE-ID:20261005T090000Z\r\nX-MICROSOFT-CDO-BUSYSTATUS:OOF\r\n",
    );
    assert!(filtered_events(&(master + &absence)).is_empty());
}
#[test]
fn exchange_vtimezone_and_quoted_windows_timezone_are_recognized() {
    let feed = "BEGIN:VCALENDAR\r\nMETHOD:PUBLISH\r\nPRODID:Microsoft Exchange Server 2010\r\nVERSION:2.0\r\nX-WR-CALNAME:Calendar\r\nBEGIN:VTIMEZONE\r\nTZID:W. Europe Standard Time\r\nBEGIN:STANDARD\r\nDTSTART:16010101T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:synthetic-outlook\r\nDTSTART;TZID=\"W. Europe Standard Time\":20261005T090000\r\nDTEND;TZID=\"W. Europe Standard Time\":20261005T100000\r\nSUMMARY:Synthetic meeting\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let c = Config::default();
    let w = c.window("2026-10-05T12:00:00Z".parse().unwrap()).unwrap();
    let events = parse_calendar(feed, &c, w).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].timestamp.to_rfc3339(),
        "2026-10-05T07:00:00+00:00"
    );
}
#[test]
fn unknown_timezone_diagnostic_names_the_missing_mapping_without_urls() {
    let c = Config::default();
    let err = c.timezone("Unknown Standard Time").unwrap_err();
    assert!(err.contains("Unknown Standard Time"));
    assert!(!c
        .timezone("https://example.invalid/private")
        .unwrap_err()
        .contains("example.invalid"));
}

#[test]
fn additional_outlook_zones_work_with_older_config_aliases() {
    let mut c = Config::default();
    for (windows, iana) in [
        ("China Standard Time", "Asia/Shanghai"),
        ("Eastern Standard Time", "America/New_York"),
        ("GTB Standard Time", "Europe/Bucharest"),
        ("Sri Lanka Standard Time", "Asia/Colombo"),
    ] {
        c.timezone_aliases.remove(windows);
        assert_eq!(c.timezone(windows).unwrap().name(), iana);
    }
    c.timezone_aliases
        .insert("China Standard Time".into(), "Asia/Hong_Kong".into());
    assert_eq!(
        c.timezone("China Standard Time").unwrap().name(),
        "Asia/Hong_Kong"
    );
}
