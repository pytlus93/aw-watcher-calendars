# aw-calendar-sync

A Rust iCalendar importer launched by ActivityWatch's tray **Modules** menu.
It reads a published Outlook HTTPS ICS URL (or a local ICS file), checks once
at startup and then every **60 minutes**, and imports **only completed events
that started in the last seven days**. This bounded catch-up window covers
overnight gaps and short outages; unchanged meetings are skipped.

**Bounded reconciliation:** changed meetings are replaced; cancelled, removed,
or filtered meetings are deleted within the import window. Older history stays
untouched. Only this module's dedicated bucket is modified; buckets are never
deleted. UID plus original recurrence start identifies a meeting. Calendar
events describe scheduled meetings, not proof of attendance or measured calls.
Set `reconcile: false` to retain the original append-only behavior.

No Google account, Microsoft Graph application, Python, Task Scheduler task,
or separate Windows service is needed. The only outbound request is downloading
your calendar feed; AW writes are restricted to a loopback HTTP server. Your
ICS URL stays in a local configuration file and is never sent to AW or logged.
Attendees are omitted by default; descriptions, locations and meeting links
are not imported.

`excluded_busy_statuses` defaults to `["OOF"]`, excluding out-of-office events
from new imports. Use `[]` to disable status filtering or add statuses such as
`FREE` or `TENTATIVE`. Outlook's actual
`X-MICROSOFT-CDO-BUSYSTATUS:OOF` takes precedence over intended `BUSY`.
If actual status is absent, intended status is used; recurrence exceptions
inherit missing status from their series. Explicit non-OOF exceptions remain
eligible. With reconciliation enabled, existing filtered records within the
active window are removed on the next successful sync.

## Build and package

Requires Rust and the MSVC Windows linker. From this folder:

```powershell
./scripts/build.ps1 -Test -Package
```

The ZIP is `dist/aw-calendar-sync-0.1.0-windows-x64.zip`.
This is a new implementation, not copied source from `aw-import-ical`.

## Install into ActivityWatch

Exit ActivityWatch before installation. In the source checkout:

```powershell
./scripts/install.ps1 -ActivityWatchDirectory 'C:\path\to\activitywatch'
```

Or extract the ZIP, open its `aw-calendar-sync` folder, and run:

```powershell
./install.ps1 -ActivityWatchDirectory 'C:\path\to\activitywatch'
```

The supplied directory must contain `aw-qt.exe`. The installer copies the
executable into `aw-calendar-sync/` beneath that directory and adds
`aw-calendar-sync` to the existing `autostart_modules` array in `aw-qt.toml`,
preserving the existing modules. It backs up an existing tray config to
`aw-qt.toml.before-calendar-sync`. Use `-NoAutostart` to install without changing
the tray configuration.

Open the config path printed by the installer and replace `calendar_url`
with your private, direct Outlook HTTPS ICS link. Keep `calendar_file` null.
Do not commit the link or paste it into bug reports. Start ActivityWatch again;
**Modules → aw-calendar-sync** starts/stops the module.

On Windows the default config is:

```text
%LOCALAPPDATA%\activitywatch\activitywatch\aw-calendar-sync\config.json
```

This matches AW's platformdirs layout. `AW_PROFILE` selects a separate
`activitywatch-<profile>` root. Logs are under the AW root's
`Logs/aw-calendar-sync/aw-calendar-sync.log` (rotated after 1 MiB), with counts
and errors identifying unsupported timezone names without feed URLs or meeting
details. With an explicit `--config` path, logs stay beside that config. The module reloads
the config before each check; restart it for immediate changes. Invalid or
unconfigured settings are retried every minute without exiting, so the module
can remain managed by AW while you finish setup.

For manual setup, place the executable in the folder described above, run
`aw-calendar-sync.exe --init-config`, edit the generated config, and add its
name to `[aw-qt].autostart_modules` through AW's **Open config folder**.
Checking it in the Modules menu alone starts it for that session; persistent
startup is controlled by `autostart_modules`.

## Configuration

See `config.example.json`. Defaults:

| Setting | Meaning |
|---|---|
| `calendar_url` / `calendar_file` | Exactly one source; HTTPS feed or local UTF-8 ICS file. Relative files resolve beside the config. |
| `calendar_id` | Stable source name and bucket suffix. Keep it unchanged to preserve duplicate detection. |
| `poll_seconds` | 3600; check once per hour while AW and this module are running. |
| `lookback_days` | Default 7 = bounded catch-up window; 1..7 = rolling days; 0 = strict today from local midnight. Never imports a future or ongoing event. |
| `server_url` | `http://127.0.0.1:5600`; only loopback origins accepted. |
| `default_timezone` | `Europe/Prague`; used for today, floating times and date-only events. |
| `timezone_aliases` | Outlook Windows timezone names mapped to IANA zones. Add mappings if needed. |
| `include_attendees` | false; opt in to storing attendee values locally in AW. |
| `excluded_busy_statuses` | `["OOF"]`; case-insensitive Outlook show-as statuses to exclude. Empty array disables filtering. Actual status takes precedence over intended status. |
| `reconcile` | true; replace changed records and remove missing/cancelled/filtered records inside the active window. false retains immutable, append-only snapshots. |

The window includes only complete events whose **start** is within the window
and whose **end** is no later than the check time. Cross-midnight meetings and
all-day events won't be imported in today-only mode once their start is before
today; use the seven-day option if those are required. Nothing already stored
is pruned as the import window moves forward.

The default seven-day window catches meetings missed before midnight or during
short shutdowns/server outages. It never expands the calendar's entire recurring
history; only occurrences in the bounded window are materialized. The entire
downloaded ICS must still be scanned, since Outlook controls the published feed
range. An outage longer than seven days can leave older meetings unimported.
Optional strict today-only mode (`lookback_days: 0`) disables overnight catch-up.

## Verify without writing to AW

```powershell
.\aw-calendar-sync.exe --dry-run
.\aw-calendar-sync.exe --once
.\aw-calendar-sync.exe --config 'C:\private\config.json' --dry-run
.\aw-calendar-sync.exe --testing --once
```

`--dry-run` downloads/parses and prints eligible event counts without contacting
AW. `--once` imports once and returns a nonzero status on failure. `--testing`
uses port 5666 and, by default, a separate `config-testing.json`. AW passes this
flag when launching modules in test mode. The default long-running mode retries
download/server failures on the next hourly check. It does not download cached
stale feed data or silently disable TLS certificate checking.

## Parsing and failure behavior

Supports RRULE, RDATE, EXDATE, moved/cancelled RECURRENCE-ID exceptions, sequence
revisions, folded lines, UTC/IANA/mapped Outlook timezones, floating times,
all-day events, DTEND and positive DURATION. Recurrences expand with bounded
iteration. All-day durations preserve local dates across DST.

Custom VTIMEZONE definitions are not interpreted; their TZID must be recognized
or explicitly mapped. EXRULE, RDATE periods and RANGE=THISANDFUTURE exceptions
fail with a redacted error, as do unknown timezone names and DST-gap date values.
No writes occur if parsing fails. HTTP redirects are rejected; use the direct
Outlook HTTPS feed URL. The feed is capped at 16 MiB and AW responses at 64 MiB.

The adapter reads retained records before each import, including records outside
the current window, to recognize moved occurrences. It refuses to import if the
bucket belongs to another client, its schema/source differs, or it contains more
than 50,000 records. Only the dedicated `aw-calendar-sync_<calendar_id>` bucket
is used. Another instance using the same config is prevented by a file lock;
use one module/config per source to avoid concurrent inserts.

Replacements are inserted and confirmed before originals are deleted. If a
reply is lost or a deletion fails, the next check rereads AW and finishes the
operation without inserting another replacement. An interrupted replacement
can briefly leave both versions present. Individual DELETE requests are verified
by reading the event back and requiring 404; no bucket deletion is implemented.
The successfully parsed feed is authoritative within the window: missing events
are removed. Use a complete feed/file for this source, not a partial export.

## Event schema (v1)

Bucket type: `calendar`, client: `aw-calendar-sync`.
Timestamp is the original scheduled start in UTC; duration is seconds to the
scheduled end. `data` contains `schema_version`, `calendar_id`, `uid`,
`occurrence` (original occurrence start, stable for moved meetings), `title`,
`all_day`, `status`, and optional `attendees`.

Calendar import adds raw events to AW. It does not add a new dashboard page or
merge meetings into tracked active time; these are inputs for the planned review
UI. The library (`src/lib.rs`) provides plain events and append-only planning;
the AW REST adapter is separate (`src/adapter.rs`).

## Validation

`cargo test --locked` covers time windows, completed/future filtering, local
midnight, recurrence exceptions, Outlook timezone mappings and DST, revisions,
append-only deduplication, malformed feeds, and real loopback HTTP requests,
including failed/lost replies and recovery. Actual Outlook feed compatibility
and launching from an installed AW tray require local runtime validation.
