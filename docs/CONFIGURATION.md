# Calendar configuration and reconciliation

## Configuration

The module supports multiple calendars in a single configuration. Shared
settings remain at the top level. Each entry has a stable `calendar_id`, exactly
one `calendar_url` or `calendar_file`, and optional `enabled` (defaults to true).

```json
{
  "poll_seconds": 3600,
  "lookback_days": 7,
  "import_scope": "completed",
  "excluded_busy_statuses": ["OOF"],
  "reconcile": true,
  "calendars": [
    {"calendar_id": "outlook-work", "calendar_url": "https://example.invalid/work.ics"},
    {"calendar_id": "team", "calendar_url": "https://example.invalid/team.ics", "excluded_busy_statuses": ["OOF", "FREE"]}
  ]
}
```

Each calendar uses its own `aw-watcher-calendars_<calendar_id>` bucket. Event UIDs
may overlap across calendars without collision. Keep existing IDs unchanged
when adding sources; changing an ID creates a new bucket. Entries can override
`excluded_busy_statuses`, `reconcile`, `lookback_days`, `default_timezone`, and
`include_attendees`, `payload_mode`, `include_summary`, `include_all_day`, and
`include_status`, and `import_scope`; other settings are shared. Polling is hourly by default
for all enabled calendars. Logs include the source's calendar ID, never its URL.

One feed's download, parse, or AW failure does not stop other valid sources.
Malformed overall configuration and duplicate IDs stop the cycle before writes.
`--once` and `--dry-run` attempt all enabled sources and exit nonzero if any fail.
Disabled or removed sources retain all their AW history: the module does not
reconcile a bucket whose source is not enabled. Up to 32 configured sources are
accepted; disable unused entries with `enabled: false`.

The old single-calendar format remains supported. To migrate, move its
`calendar_id` and source URL/file into the first `calendars` entry; retain all
other settings at the top level. The optional migration helper does this
atomically without displaying the URL:

```powershell
python .\scripts\migrate-config.py 'C:\private\config.json'
```

The helper needs Python only for migration; the AW module itself does not.
Use `aw-watcher-calendars.exe --validate-config` to check the configuration without
fetching calendars or connecting to AW.
Never leave top-level source fields alongside the `calendars` array.

See [config.example.json](../config.example.json). Defaults:

| Setting | Meaning |
|---|---|
| `calendars[].calendar_url` / `calendar_file` | Exactly one source; HTTPS feed or local UTF-8 ICS file. Relative files resolve beside the config. |
| `calendars[].calendar_id` | Unique stable source name and bucket suffix. Keep it unchanged to preserve duplicate detection. |
| `calendars[].enabled` | true; false suspends this source while preserving its stored history. |
| `poll_seconds` | 3600; check once per hour while AW and this module are running. |
| `import_scope` | `completed` (default), `started`, or `current_day`; shared or per-calendar. |
| `lookback_days` | Default 7 = bounded catch-up window; 1..7 = rolling days; 0 = today from local midnight. Used by completed/started; ignored by current_day. |
| `server_url` | `http://127.0.0.1:5600`; only loopback origins accepted. |
| `default_timezone` | `Europe/Prague`; used for today, floating times and date-only events. |
| `timezone_aliases` | Outlook Windows timezone names mapped to IANA zones. Add mappings if needed. |
| `payload_mode` | New templates: strict custom replay schema plus module title; existing configs without this key: extended, preserving old payloads. No official calendar schema is claimed. |
| `include_summary` | true; false replaces SUMMARY with the neutral mandatory title. |
| `include_all_day` / `include_status` | Individually selectable extensions, emitted only in extended mode. Missing flags default true for compatibility. |
| `include_attendees` | false; opt in to storing nonempty attendee values locally in AW, only in extended mode. |
| `excluded_busy_statuses` | `["OOF"]`; case-insensitive Outlook show-as statuses to exclude. Empty array disables filtering. Actual status takes precedence over intended status. |
| `reconcile` | true; replace changed records and remove missing/cancelled/filtered records inside the active window. false retains immutable, append-only snapshots. |

## Import scope

| Value | Included events | Boundaries |
|---|---|---|
| `completed` | Backwards, ended events only | Starts within lookback; scheduled end no later than poll time |
| `started` | Backwards, including currently running | Starts within lookback and no later than poll time; keeps full scheduled end |
| `current_day` | Whole current day, including future meetings | Starts from local midnight inclusive to next midnight exclusive; keeps full scheduled end |

Use **Settings… → Shared import scope**, then Save. The selector updates the
top-level `import_scope` key in the editor; overrides inside individual calendars
remain intact and must be edited there. Manual JSON is also supported, for example
`"import_scope": "started"`. Missing keys preserve the completed-only default;
no migration changes existing live choices or silently enables future imports.

All dates use each calendar's `default_timezone`. Current-day boundaries follow
local DST (23/25-hour days) and ignore `lookback_days`; this scope provides no
previous-day catch-up. It includes all-day meetings and meetings ending tomorrow
when they start today, but excludes meetings starting yesterday or next midnight.
Use a backwards scope with lookback >0 for cross-midnight catch-up.

Running/future intervals represent the **full published schedule**, not elapsed
time or foreground work. Their duration does not grow each poll. Status filters
(including OOF), payload privacy, polling and source/bucket identity are unchanged.
Reconciliation applies to records in the selected scope: edits and cancellations
can replace/remove running/future intervals when that scope includes them.
Older/out-of-scope records stay frozen. Switching to a narrower scope does not
erase previously stored future intervals outside the new scope. Append-only
`reconcile:false` retains the original snapshots regardless of later feed edits.

The default seven-day window catches meetings missed before midnight or during
short shutdowns/server outages. It never expands the calendar's entire recurring
history; only occurrences in the bounded window are materialized. The entire
downloaded ICS must still be scanned, since Outlook controls the published feed
range. An outage longer than seven days can leave older meetings unimported.
Optional strict today-only mode (`lookback_days: 0`) disables overnight catch-up.

## Validation and one-shot commands

```powershell
.\aw-watcher-calendars.exe --dry-run
.\aw-watcher-calendars.exe --once
.\aw-watcher-calendars.exe --config 'C:\private\config.json' --dry-run
.\aw-watcher-calendars.exe --testing --once
```

`--dry-run` downloads/parses and prints eligible event counts without contacting
AW. `--once` imports once and returns a nonzero status on failure. `--testing`
uses port 5666 and, by default, a separate `config-testing.json`. AW passes this
flag when launching modules in test mode. The default long-running mode retries
failed cycles after at most 60 seconds; successful cycles keep the hourly cadence.
It does not download cached
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
than 50,000 records. Only the dedicated `aw-watcher-calendars_<calendar_id>` bucket
is used. Another instance using the same config is prevented by a file lock;
use one AW-managed module/config containing all sources to avoid concurrent inserts.

Replacements are inserted and confirmed before originals are deleted. If a
reply is lost or a deletion fails, the next check rereads AW and finishes the
operation without inserting another replacement. An interrupted replacement
can briefly leave both versions present. Individual DELETE requests are verified
by reading the event back and requiring 404, or confirming absence in the full
record list after Rust-server lookup 500; no bucket deletion is implemented.
The successfully parsed feed is authoritative within the window: missing events
are removed. Use a complete feed/file for this source, not a partial export.

## Event schema (v1)

Bucket type: `calendar`, client: `aw-watcher-calendars`.
Timestamp is the original scheduled start in UTC; duration is seconds to the
scheduled end. `data` contains `schema_version`, `calendar_id`, `uid`,
`occurrence` (original occurrence start, stable for moved meetings), `title`,
optional `all_day`, `status`, and `attendees`, depending on payload mode/settings.
The complete [data contract](data-contract.md) documents every field,
privacy/default/unavailable behavior, migration and ordering limitations.
There is no official standard `calendar` bucket contract. Strict here means the
required **custom** replay fields plus required `title`; official-only strict
semantics and dedicated settings selectors remain open requirements. A native
configuration/privacy JSON editor is now available through the tray actions protocol.

Calendar import adds raw events to AW. It does not add a new dashboard page or
merge meetings into tracked active time; these are inputs for the planned review
UI. The library (`src/lib.rs`) provides plain events and append-only planning;
the AW REST adapter is separate (`src/adapter.rs`).
