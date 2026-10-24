use aw_watcher_calendars::{parse_calendar, plan_reconcile, Config, StoredEvent, Window};
fn window() -> Window {
    Window {
        start: "2026-10-05T00:00:00Z".parse().unwrap(),
        end: "2026-10-05T12:00:00Z".parse().unwrap(),
    }
}
fn event() -> aw_watcher_calendars::CalendarEvent {
    let feed="BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:synthetic\r\nSUMMARY:Original\r\nDTSTART:20261005T090000Z\r\nDTEND:20261005T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    parse_calendar(feed, &Config::default(), window())
        .unwrap()
        .remove(0)
}
#[test]
fn reconciliation_replaces_changes_and_removes_missing_only_in_window() {
    let original = event();
    let old = StoredEvent {
        id: 1,
        event: original.clone(),
    };
    let mut changed = original.clone();
    changed.data.title = "Changed".into();
    let plan = plan_reconcile(&[changed.clone()], std::slice::from_ref(&old), window());
    assert_eq!(plan.insert, vec![changed]);
    assert_eq!(plan.delete, vec![1]);
    assert_eq!(plan.replaced, 1);
    assert_eq!(
        plan_reconcile(&[], std::slice::from_ref(&old), window()).delete,
        vec![1]
    );
    let mut historical = old.clone();
    historical.event.timestamp = "2026-10-04T09:00:00Z".parse().unwrap();
    let plan = plan_reconcile(&[original], &[historical], window());
    assert!(plan.insert.is_empty());
    assert!(plan.delete.is_empty());
}
#[test]
fn interrupted_replacement_retains_matching_copy_and_removes_duplicate() {
    let original = event();
    let mut changed = original.clone();
    changed.data.title = "Changed".into();
    let existing = vec![
        StoredEvent {
            id: 1,
            event: original,
        },
        StoredEvent {
            id: 2,
            event: changed.clone(),
        },
    ];
    let plan = plan_reconcile(&[changed], &existing, window());
    assert!(plan.insert.is_empty());
    assert_eq!(plan.delete, vec![1]);
    assert_eq!(plan.unchanged, 1);
}
#[test]
fn ongoing_future_and_older_records_are_never_pruned() {
    let mut records = Vec::new();
    for (id, time, duration) in [
        (1, "2026-10-04T09:00:00Z", 3600.0),
        (2, "2026-10-05T14:00:00Z", 3600.0),
        (3, "2026-10-05T11:00:00Z", 7200.0),
    ] {
        let mut e = event();
        e.timestamp = time.parse().unwrap();
        e.duration = duration;
        records.push(StoredEvent { id, event: e });
    }
    assert!(plan_reconcile(&[], &records, window()).delete.is_empty());
}
