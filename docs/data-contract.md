# Calendar data contract

## Official AW contract and this module's custom schema

The [AW data model](https://docs.activitywatch.net/en/latest/buckets-and-events.html)
requires event `timestamp` (RFC 3339 UTC), `duration` (seconds) and object `data`.
The documented standard bucket types do **not** include a calendar schema.
`calendar`, client `aw-calendar-sync`, is this module's custom type. Do not call
its identity properties official AW standard fields or infer dashboard support.
Events describe scheduled calendar intervals, not measured foreground work,
attendance or calls. Foreground/idle gating would discard valid imported history
and is inapplicable; no heartbeats are sent and gaps are never merged.

The [REST reference](https://docs.activitywatch.net/en/latest/api/rest.html)
defines list POSTs for events and `304` for existing bucket creation. Creation is
followed by metadata validation, including after 304. A 304 elsewhere is an error.
Server versions differ: when single-event lookup returns 500 after deletion,
the complete event list must independently confirm absence. A generic 500 alone
is never considered proof of deletion. See also the official
[watcher guide](https://docs.activitywatch.net/en/latest/watchers.html) and
[watcher example](https://docs.activitywatch.net/en/latest/examples/writing-watchers.html).

## Modes and selectable content

`payload_mode: "strict"` sends **this custom schema's required identity fields**
and the workspace-required `title`; it omits all three optional extensions.
This is a restricted custom-schema mode, **not** a claim to implement an official
standard calendar contract. An interpretation of strict as only official AW
data fields plus title remains unsupported: removing the identity fields would
break recurrence identity and safe replay/reconciliation. A future implementation
would require a separate durable identity journal and its own migration/recovery.

`payload_mode: "extended"` adds individually selected `all_day`, `status` and
`attendees`. `include_summary` independently controls whether title contains the
published SUMMARY. `title` remains mandatory when summary is disabled.
All settings can be shared or overridden in an individual `calendars` entry.

| Configuration | New template | Missing setting in an existing config | Effect |
|---|---|---|---|
| `payload_mode` | `strict` | `extended` | Preserves old optional payloads without silently adding metadata |
| `include_summary` | true | true | Use SUMMARY for title; false uses neutral fallback |
| `include_all_day` | true (inactive in strict) | true | Emit `all_day` only in extended mode |
| `include_status` | true (inactive in strict) | true | Emit `status` only in extended mode |
| `include_attendees` | false | false | Emit nonempty attendee list only when extended and opted in |

The example config explicitly sets optional flags false. `--init-config` uses
strict mode; its all-day/status flags are inactive until extended is selected.
The existing calendar-list migration preserves all original settings. No live
config is rewritten by mode introduction, and attendee collection stays off.
Settings are currently edited through AW's **Open config folder**. An integrated
AW settings form with selectors is **not implemented**, so the settings-UI
requirement remains open. No independent settings executable is deployed.

## Every emitted `data` field

Examples are synthetic. Required custom fields are always enabled. Optional
disabled properties are absent, not null/empty placeholders.

| Field | Type and meaning | Example | Applicability/default | Privacy | Unavailable behavior |
|---|---|---|---|---|---|
| `schema_version` | integer; custom replay schema | `1` | All events, required | No calendar content | Always generated; incompatible versions fail closed |
| `calendar_id` | string; configured source/bucket identity | `work` | All events, required | User-chosen source label; also occurs in bucket and counts-only logs | Invalid/missing config uses documented default or fails validation |
| `uid` | string; original ICS UID | `synthetic-uid` | All events, required | Stable correlation ID; some publishers embed email/host identifiers | Missing or empty UID fails feed before writes |
| `occurrence` | string; original occurrence start in UTC, unchanged on moves | `2026-10-05T09:00:00+00:00` | All events, required | Reveals original scheduled time even when moved | Derived from DTSTART/RECURRENCE-ID; invalid date fails feed |
| `title` | string; concise summary, whitespace collapsed, max 200 Unicode characters | `Deployment review` | All events; summary enabled by default | May reveal meeting topic, names or customer context from SUMMARY | Empty/missing SUMMARY or `include_summary:false` yields `Meeting (title unavailable)` |
| `all_day` | boolean; date-only calendar interval | `false` | Extended + `include_all_day:true`; legacy default on | Reveals scheduling category | Derived from DTSTART VALUE/date; malformed dates fail feed |
| `status` | string; ICS STATUS (separate from Outlook show-as filter) | `TENTATIVE` | Extended + `include_status:true`; legacy default on | Reveals scheduling status | Missing STATUS defaults to `CONFIRMED`; cancelled events are excluded |
| `attendees` | array of strings; raw ATTENDEE values (usually mailto URIs) | `["mailto:synthetic@example.invalid"]` | Extended + `include_attendees:true`; default off | Personal/contact data; explicit opt-in | Missing/empty list omitted |

Descriptions, location, organizer, meeting links, feed URL and raw ICS text are
never fields or title inputs. Titles receive no module/application prefix.
Logs contain IDs, counts and redacted errors, never titles/attendees/URLs. Unknown
timezone names may be logged after length/control/URL checks; use synthetic
timezone labels if a source includes confidential identifiers.

Disabled extensions never feed titles or logs. Source identity is mandatory and
is not selectable; this must not be described as metadata-free/title-only capture.
Changing payload settings affects future syncs. With reconciliation enabled,
completed records inside the seven-day window are replaced with the selected
payload. Older frozen history and append-only mode retain previously stored
content; this importer does not provide full-history privacy erasure or exports.

## Ordering and display evidence

Struct serialization emits required custom properties in the table's order,
workspace `title` next, then enabled extensions. The transport serializes structs
directly, rather than sorting keys through `serde_json::Value`. Synthetic tests
check actual received request bytes. JSON key order has no semantic significance.
AW can reorder keys during storage/API serialization; browser tooltips can apply
their own ordering. No storage/tooltip ordering guarantee is made. A future display
ordering fix belongs in AW's presentation layer, not payload insertion order.
