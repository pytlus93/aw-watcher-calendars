# Validation evidence — 2026-10-11

Scope: calendar module only, against the current parent AGENTS.md. Shared root
documentation, live executables/configs and production calendar events were not
changed. No live watchers were killed, no replacement was installed, no interactive
test UI was opened, and no PC restart was attempted.

## Build and synthetic evidence

Toolchain: Rust/cargo 1.99.0, Windows/MSVC target, Python 3.14.7.

| Check | Result | Evidence boundary |
|---|---|---|
| `cargo test --locked` | PASS: 43 tests | 42 original regressions plus validated atomic configuration save |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS | All targets; no suppressed new warnings |
| `./scripts/build.ps1 -Package` | PASS | Release executable and ZIP; not installed into production |
| `python tests/migration.py` | PASS: 2 tests | Preserves private source/settings and original bytes on invalid input; idempotent |
| `./tests/install-smoke.ps1` | PASS: 5 variants, repeated installation | Synthetic AW folder/config under ignored local; no production installation |
| `python tests/lifecycle.py` | PASS: 2 tests, about 66 seconds | Real module child processes and isolated loopback server |
| `python tests/actions.py` | PASS: 6 tests | Protocol dispatch/context, safe reports, producer singleton bypass and manifest/release/ZIP consistency; headless only |
| `python ../aw-qt/tests/test_recovery_standalone.py` | PASS: 5 tests | Current sibling host-source policy with synthetic module objects; not installed-host evidence |

HTTP regressions use real sockets and a synthetic AW-like server: first bucket
creation; an existing-bucket race returning 304 followed by metadata verification;
fresh adapter restart/replay; genuine 503 creation/insert/delete failures; rejection
of event-POST 304; lost insert response; count acknowledgements verified by
readback; interrupted replacement/deletion recovery; 500 absent-event lookup
confirmed through the event list; outage/restart and actual record reinsertion.
Server-storage-loss simulation recreates bucket/data while preserving original
calendar timestamps and durations. It does not claim a real AW service restart.

The executable test starts before its isolated server exists. Initial connection
failure is logged, the server starts, and the **same running producer** retries
and delivers two distinct 30-minute intervals with a 90-minute scheduled gap.
`poll_seconds` stays 3600; failures trigger a bounded one-minute retry.
The test captures actual HTTP request bytes, confirms custom identity properties
precede title and strict extensions are absent. The fake server does not establish
real AW storage or GUI ordering.

Lifecycle child-process test: second instance with the same config exits nonzero;
the harness kills only its own child to simulate a crash; the OS releases its lock;
a replacement launches; explicit termination stops that child. No module self-spawns.
This confirms lock/relaunch mechanics, not automatic installed AW host recovery.
Sibling host policy tests cover crash restart, failed relaunch that does not block
other modules, intentional stop, bounded 5..300-second backoff and stable-run reset.
Host source currently wires policy checks into the tray timer. Deployment of that
host source was not established by this validation.

Privacy regression changes a stored extended event to restricted custom strict
mode with summary disabled: confirmed replacement then deletion removes the old
in-window title/attendees/status/all-day values; repeat sync is unchanged.
No description/location/link is emitted. Older history and append-only mode are
deliberately frozen and are not a full-history privacy-erasure mechanism.

## Read-only deployed-server evidence

The running server's `/api/0/info` reports **v0.13.2**, testing false.
`/api/swagger.json` exposes `/0/buckets/{bucket_id}` and its events endpoint plus
the `Event` model. Its creation schema lists 200 only, while maintained official
documentation also specifies 304 for an existing bucket. Tests therefore cover
304 explicitly rather than assuming the deployed Swagger list is exhaustive.
Only info/schema were read: no live event payloads or feed URLs were printed,
no production source was downloaded, and no live calendar writes were performed.

Earlier user-session evidence exists for installed tray launch and receipt of
calendar records (including bounded OOF removal and replay). Those observations
predate these changes and are not validation of this release, reboot recovery,
payload selections or presentation order.

Sources reviewed: [data model](https://docs.activitywatch.net/en/latest/buckets-and-events.html),
[REST](https://docs.activitywatch.net/en/latest/api/rest.html),
[watchers](https://docs.activitywatch.net/en/latest/watchers.html),
[watcher example](https://docs.activitywatch.net/en/latest/examples/writing-watchers.html),
and maintained ureq 2.12.1 `send_json` implementation (direct serde byte serialization).

## Remaining requirements / unperformed checks

| Requirement | Status and next evidence needed |
|---|---|
| Host settings UI for per-field selections | PARTIAL: native configuration/privacy JSON editor via host-owned tray action is implemented; dedicated selectors and visual validation remain open |
| Official-only strict data fields plus title | MISSING: no official calendar schema exists; current strict preserves documented mandatory custom replay identity. Removing it safely needs a durable identity journal/migration |
| Windows login after hard PC restart | UNPERFORMED: requires authorized reboot/login observation; autostart config alone is insufficient |
| Installed host automatic crash/failed-relaunch recovery | UNPERFORMED: policy/source tests pass; production process disruption was prohibited |
| Installed host intentional stop/no duplicates across login | UNPERFORMED: isolated executable and policy tests pass, live tray/login behavior not tested |
| Actual AW storage/API key order and tooltip support | UNPERFORMED: wire bytes checked; real server storage and GUI can reorder keys; display fix belongs in AW UI |
| New payload modes on production calendars | UNPERFORMED: synthetic content tested; no production update or feed access in this pass |

The module remains an interval importer. Foreground/idle heartbeat recovery is
inapplicable: imports retry fixed scheduled intervals within the bounded window,
never fabricate active-work observations or extend through idle/outage gaps.

## Tray actions integration (same-day follow-up)

Protocol 1 adds Configure sources/privacy, Connection/last sync status, Safe logs
and About actions. Shared config/profile/testing resolution and explicit CLI
server overrides are covered. Helpers dispatch before producer locking/collection;
unknown commands and invalid protocol/action identity fail without starting a
producer. Tests run helpers alongside an isolated real producer and verify a
second producer is still rejected. Valid info and unavailable-server reports are
honest and no feed access or event write is performed by management commands.

At the initial integration, native UI compiled/linked but was not opened.
Headless configuration checks do not test GUI
interaction; the separate unit test exercises its validated atomic save function.
The installer smoke test additionally checks sidecar copies and stale-sidecar
removal for rollback input without an adjacent manifest. Lifecycle was rerun and
passed two tests in about 63 seconds. Migration tests still pass. Original tray
compatibility is source/package design evidence, not a new installed-tray test.
No IPC/Refresh command is advertised. No live deployment or GitHub workflow was
performed. See [action protocol documentation](module-actions.md) for commands,
privacy-safe separate action logs and rollback instructions.

## Native configuration follow-up — 2026-10-11

The user reported the installed calendar action as crashing. Installed and local
release SHA-256 matched. With synthetic configuration, both normal launches stayed
alive and normal-launch Save worked. No matching application-fault event was found.
The reproduced defect is specifically an **invisible editor under the tray's
hidden startup flag**, not a demonstrated exception or process crash.

The installed baseline's real native visibility regression fails after five
seconds: its `AwCalendarConfig` window exists but `IsWindowVisible` remains false
with STARTF_USESHOWWINDOW/SW_HIDE. The fixed editor is built before it is explicitly
shown, consumes the startup flag and then shows itself. Save/error dialogs now have
the editor as owner and disable it while modal. The edit text limit is explicitly
131072 characters to match Save validation, and message-loop failures are errors.

This startup behavior is documented by Microsoft's
[ShowWindow reference](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow).

Local evidence: 43 Rust tests and warning-denied clippy pass; release/ZIP builds;
all six headless action tests pass. `python tests/native_actions.py -v` passes
three **real native UI** tests on release and the staged-package executable:
visible initial editor text and Save/Cancel controls, Cancel without writes,
invalid-edit Save preserving original bytes, an owned modal error dialog followed
by a valid >32 KiB Save, owned modal confirmation, close without writes, zero exit,
no producer singleton/collection. The harness uses actual Win32 control messages,
not helper headless mode or a mock UI. Config/profile data are private-isolated and
synthetic; no live config contents were read or edited. Only isolated action children
were closed/terminated. Read-only live config size and binary hashes were checked.

Pixel-level visual layout/signoff, a real installed-tray click, deployed fixed
binary, live private-config interaction and reboot remain unperformed. The staged
fixed package is ready for the coordinating chat to deploy; this component did not
install or stop a production module. Native tests are opt-in with explicit UI
authorization. Prior lifecycle and transport evidence remains separate.

Coordinator-reported follow-up: the coordinating chat backed up the previous
module, deployed the fixed dist executable plus matching sidecar and restarted
the tray. It reports all three native-action tests passed against the **installed
executable with synthetic settings** and tray/calendar/VPN processes are running.
This component did not independently repeat that deployment or installed test.
This is narrow installed-binary synthetic UI evidence, not proof of a user's
actual tray click with live settings, successful calendar delivery, reboot or
host recovery. The real user tray/live-settings retry remains pending.


## Fresh validation — 2026-10-16

Fresh offline locked Rust suite PASS (43 tests), all-target Clippy with warnings denied and release packaging PASS. Python migration (2), lifecycle (2), and actions (6) tests passed; installer smoke passed five configurations with repeated installation. Tests use synthetic isolated settings and owned processes. Production calendar bucket contains positive-duration records dated today. Installed executable SHA256 matches the fresh release build. Receipt does not prove source-calendar correctness or every reconciliation scenario. No private feed/config/content was printed or committed; no production restart, UI or reboot was exercised.

## Selectable import scopes — 2026-10-19

Added `import_scope` with completed (unchanged default), started (including running
meetings) and current_day (events starting in the whole local day, including
future meetings). Both backwards scopes retain 0..7-day lookback; current_day
uses local midnight to exclusive next midnight. Full scheduled durations are
retained, including meetings crossing midnight; no elapsed work is inferred.
Reconciliation and clock checks follow the scope, with older/out-of-scope records
frozen. Shared and per-calendar config overrides are validated before writes.

Local checks PASS: 50 Rust tests; warning-denied all-target clippy; release and ZIP
packaging; migration (2), lifecycle (2), headless actions (6), native UI (4), and
installer smoke (five variants, repeated installation plus sidecar rollback).
The native tests used the previously authorized isolated synthetic settings and
real Windows controls under hidden-startup protocol context. They selected all
three dropdown values, checked JSON updates and per-calendar override preservation,
and saved; earlier visibility, Cancel, invalid-edit and close cases still pass.
No live config, source, installed executable or running production process was
changed by the feature implementation. No current-day/future interval was written
to production AW. Existing settings still default to completed until explicitly
changed after installing the updated build.

New core/HTTP cases cover completed/running/future filtering, exclusion of next
midnight, both DST transitions, all-day and recurring events, overnight catch-up,
OOF filtering, scope migration, replay and actual insert/replace/delete of ongoing
or future scheduled intervals. This is synthetic protocol evidence, not a new
live server compatibility or GUI tooltip claim. Hourly polling is unchanged.

### Bodyless existing-bucket regression

`python tests/transport.py` passes two additional real-CLI tests against a
persistent HTTP/1.1 synthetic server. Bucket POST returns **304, Content-Length:
67, no body, keep-alive**. The producer finishes without waiting for 67 bytes,
validates metadata GET, inserts a real calendar interval and confirms it. A fresh
producer repeats existing-bucket initialization and replays without duplication,
preserving timestamp and duration. Event POST 304 and incompatible bucket type
both fail with no insertion; valid initialization subsequently resumes delivery.
Normal process-exit TCP resets are caught only by this test handler, not ignored
by production transport. This test uses interval import semantics, not heartbeats.
The deployed AW implementation's equivalent bodyless response was not exercised;
a reachable info endpoint alone is not treated as proof of this compatibility.
