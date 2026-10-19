# ActivityWatch calendar sync

An AW-managed Windows module that imports scheduled events from published Outlook HTTPS iCalendar feeds or local ICS files. Choose completed events, include ongoing meetings, or import the whole current day. Multiple calendars use separate buckets. Scheduled events are context, not proof of attendance or tracked work time.

## Requirements

- ActivityWatch with a local server and `aw-qt.exe` module launcher.
- A direct HTTPS ICS feed or a local UTF-8 ICS file for each calendar.

No Microsoft Graph application or separate service is needed. Feed downloads contact their configured servers; activity is stored in local AW. Feed URLs can grant access to private calendars: keep them out of source control and reports. Attendees are off by default; descriptions, locations and meeting links are not imported.

## Install or upgrade

Extract `aw-calendar-sync-0.1.0-windows-x64.zip`, close ActivityWatch normally and run from its `aw-calendar-sync` folder:

```powershell
.\install.ps1 -ActivityWatchDirectory 'C:\path\to\activitywatch'
```

From source use `./scripts/install.ps1`. The installer preserves settings and existing startup modules, backs up the tray configuration, and adds calendar sync to autostart. Use `-NoAutostart` to preserve the startup list. Keep the matching executable/actions sidecar together when upgrading or rolling back.

Edit the config path printed by the installer, replacing the first calendar's placeholder URL with your direct private feed. Start ActivityWatch normally. **Modules → aw-calendar-sync** controls it; protocol 1 hosts also provide settings/status/log/about actions. See [tray actions](docs/module-actions.md).

## Configuration and everyday use

Default config: `%LOCALAPPDATA%\activitywatch\activitywatch\aw-calendar-sync\config.json`. Profiles use isolated roots. The module reloads settings before each check.

| Setting | Default behavior |
| --- | --- |
| `poll_seconds` | `3600`; check at startup and hourly |
| `import_scope` | `completed`; choose `completed`, `started` or `current_day` in Settings |
| `lookback_days` | `7`; backwards catch-up for completed/started scopes; ignored by current_day |
| `excluded_busy_statuses` | `["OOF"]`; omit out-of-office events |
| `reconcile` | `true`; replace changed and remove missing/cancelled/filtered events in the window |
| `include_attendees` | `false` |
| `server_url` | `http://127.0.0.1:5600`; loopback only |

Older history is retained. Because reconciliation treats a successfully parsed feed as authoritative within the window, use a complete feed for each source. `reconcile: false` keeps append-only snapshots instead.

**Settings… → Shared import scope** selects backwards completed only, backwards
including currently running, or the whole current local day (including future
meetings). Ongoing/future imports retain the **full scheduled duration**. The
first two scopes keep up to seven days of catch-up; current day includes events
starting today only. Per-calendar `import_scope` overrides are supported in JSON.
Hourly polling remains unchanged; settings take effect at the next check.

Each `calendars` entry needs a stable `calendar_id` and exactly one `calendar_url` or `calendar_file`. Changing an ID creates a different bucket; disabling/removing a source freezes its old history. See [the synthetic config template](config.example.json) and [configuration and migration](docs/CONFIGURATION.md).

New templates use strict custom replay fields plus a readable title; this is not an official AW calendar schema. Older configurations without a mode retain extended behavior. Summary capture is on; additional fields are configurable. See [the data contract](docs/data-contract.md).

## Troubleshooting

- **Unconfigured or invalid settings:** the worker stays available and retries. Validate without network access using `aw-calendar-sync.exe --validate-config`.
- **No imported events:** the default completed scope excludes future/ongoing events. Check scope, poll timing, lookback, timezone and status filters. Outages longer than seven days may leave older events unimported.
- **Feed fails:** use a direct URL; redirects and unsupported recurrence/timezone forms are rejected before writes. Logs omit feed URLs and meeting text.
- **No chart:** raw `calendar` events do not automatically add a dashboard page or become active work time.

`--dry-run` downloads/parses and reports counts without AW writes. `--once` performs an actual import. See [configuration](docs/CONFIGURATION.md) for these commands, log locations and reconciliation recovery.

## Local development

Requires Rust/MSVC; Python is needed only for auxiliary tests/migration.

```powershell
./scripts/build.ps1 -Test -Package
python tests/migration.py
python tests/lifecycle.py
./tests/install-smoke.ps1
python tests/actions.py
python tests/transport.py
```

[Validation](docs/validation.md) records parser, protocol, migration, isolated lifecycle and installed evidence. Real-host reboot recovery and interactive settings acceptance remain separate checks.

## License

[MIT](LICENSE).
