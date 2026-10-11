use aw_calendar_sync::{config::CalendarSet, parse_calendar, Config, PayloadMode, Window};

fn events(config: &Config, summary: &str) -> Vec<aw_calendar_sync::CalendarEvent> {
    let text = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:synthetic-uid\r\nDTSTART:20261005T090000Z\r\nDTEND:20261005T100000Z\r\nSUMMARY:{summary}\r\nSTATUS:TENTATIVE\r\nATTENDEE:mailto:synthetic@example.invalid\r\nDESCRIPTION:Excluded message body\r\nLOCATION:Excluded location\r\nURL:https://example.invalid/private\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
    parse_calendar(
        &text,
        config,
        Window {
            start: "2026-10-05T00:00:00Z".parse().unwrap(),
            end: "2026-10-05T12:00:00Z".parse().unwrap(),
        },
    )
    .unwrap()
}

#[test]
fn strict_omits_extensions_and_summary_can_be_disabled_without_indirect_leaks() {
    let config = Config {
        payload_mode: PayloadMode::Strict,
        include_summary: false,
        include_attendees: true,
        ..Config::default()
    };
    let event = events(&config, "Private meeting topic").remove(0);
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(event.data.title, "Meeting (title unavailable)");
    for forbidden in [
        "Private meeting topic",
        "attendees",
        "all_day",
        "status",
        "Excluded",
        "example.invalid",
    ] {
        assert!(!json.contains(forbidden));
    }
    let data = serde_json::to_value(&event.data).unwrap();
    assert_eq!(data.as_object().unwrap().len(), 5);
    let keys = [
        "schema_version",
        "calendar_id",
        "uid",
        "occurrence",
        "title",
    ];
    let offsets: Vec<_> = keys
        .iter()
        .map(|key| json.find(&format!("\"{key}\":")).unwrap())
        .collect();
    assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn extended_fields_are_individually_selectable_and_titles_are_concise() {
    let config = Config {
        include_attendees: true,
        include_all_day: false,
        include_status: false,
        ..Config::default()
    };
    let event = events(&config, &format!("Topic\\n{}", "x".repeat(250))).remove(0);
    let data = serde_json::to_value(&event.data).unwrap();
    assert!(data.get("all_day").is_none());
    assert!(data.get("status").is_none());
    assert_eq!(data["attendees"][0], "mailto:synthetic@example.invalid");
    assert_eq!(event.data.title.chars().count(), 200);
    assert!(!event.data.title.contains('\n'));
    assert_eq!(
        events(&Config::default(), "")[0].data.title,
        "Meeting (title unavailable)"
    );
}

#[test]
fn existing_config_keeps_legacy_modes_new_templates_are_strict_and_overrides_work() {
    let legacy = CalendarSet::from_json(r#"{"calendar_file":"synthetic.ics"}"#).unwrap();
    assert_eq!(legacy.calendars[0].payload_mode, PayloadMode::Extended);
    assert!(!legacy.calendars[0].include_attendees);
    let template = CalendarSet::template(false);
    assert_eq!(template["payload_mode"], "strict");
    let settings = CalendarSet::from_json(r#"{"payload_mode":"strict","calendars":[{"calendar_id":"one","calendar_file":"one.ics"},{"calendar_id":"two","calendar_file":"two.ics","payload_mode":"extended","include_summary":false,"include_status":false}]}"#).unwrap();
    assert_eq!(settings.calendars[0].payload_mode, PayloadMode::Strict);
    assert_eq!(settings.calendars[1].payload_mode, PayloadMode::Extended);
    assert!(!settings.calendars[1].include_summary);
    assert!(!settings.calendars[1].include_status);
}
