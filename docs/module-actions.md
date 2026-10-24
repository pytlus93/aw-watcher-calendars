# AW tray actions protocol 1

This module packages `aw-watcher-calendars.exe.actions.json` beside the executable.
An enhanced aw-qt reads it and displays the following submenu. An original tray
ignores it and retains the usual checkable module entry. Neither AW server needs
modification. aw-tauri does not currently consume this protocol.

| Menu action | Command | Implemented behavior |
|---|---|---|
| Settings… | `configure` | Native Windows JSON editor with Save/Cancel, privacy-setting guidance and validated atomic replacement; works with AW stopped |
| Status… | `status-ui` | Read-only, three-second loopback `/api/0/info` probe plus recent redacted counts; reports unavailable/invalid response honestly |
| Logs… | `open-logs` | Native dialog showing up to 30 newest timestamps/counts; arbitrary error text, URLs, titles, attendees and source names omitted |
| About… | `about` | Native version/capability dialog |

These are short-lived helpers in the same executable, dispatched **before** the
producer singleton lock, logging loop or calendar download. They never create
another producer, import events, launch a browser/editor/PowerShell child, detach
processes or register a separate service. The configuration window and message
dialogs belong directly to the helper process supervised by the host.
The tray owns Running/start/stop/recovery; helper exit does not imply sync success.
Unknown commands, action-ID mismatch and unsupported protocol versions fail.
There is no Refresh now action: no local IPC is implemented.

The configuration editor exposes payload/privacy choices as editable JSON keys,
not dedicated checkbox selectors. It validates the entire config before Save,
including per-calendar overrides and source IDs. It preserves invalid edits on
screen without altering the file. URLs are visible only inside the local editor;
no raw config or URL is logged. Saving does not trigger a sync; the producer reloads
on its next scheduled/retry check. Older history remains frozen by existing policy.
Malformed configurations can still be opened for repair without contacting AW.

## Launch context and manual fallback

### Import scope selector

Settings includes **Shared import scope**, a three-choice dropdown:
backwards completed only, backwards including currently running, or whole current
local day including future events. Selecting an option updates `import_scope` in
the JSON editor; Save validates and atomically persists it. Per-calendar overrides
are preserved and edited inside their entries. See [scope semantics](CONFIGURATION.md#import-scope).
Manual JSON edits remain authoritative when saved. No selection starts an import;
the worker reloads settings at its next poll. Default is completed only.

The native regression now selects each option through the real combo control,
checks generated JSON and per-calendar override preservation, then saves. Existing
hidden-startup visibility, Cancel, invalid-edit and close regressions still pass.

### Context

With `AW_MODULE_ACTION_PROTOCOL=1`, dispatch consumes and validates
`AW_MODULE_ACTION`, `AW_TESTING` (`0`/`1`), `AW_PROFILE`, and `AW_SERVER_URL`.
Action IDs are `configure`, `status`, `logs`, `about`. Profile/config resolution
is shared with producer startup. Named profiles use
`%LOCALAPPDATA%/activitywatch-<profile>/activitywatch-<profile>/aw-watcher-calendars/`;
testing selects `config-testing.json` and the corresponding testing log.
Absent/default profile uses the original activitywatch root.

Manual CLI remains supported, including `--testing`, `--config PATH` and
`--server-url URL`. Explicit CLI server override wins over action environment,
then testing/default configuration determines the server. Server origins remain
loopback HTTP only, with proxy/redirects disabled. Normal legacy producer commands
remain compatible; protocol context cannot fall through into the producer loop.

`--port <port>` (1–65535) takes precedence over those endpoint ports for both
actions and imports. Without a CLI/action server URL it selects 127.0.0.1 at the
requested port. It does not change testing/profile selection or saved settings.

One-line manual examples (run the module executable in its installation folder):

```powershell
./aw-watcher-calendars.exe configure
./aw-watcher-calendars.exe status-ui --testing
./aw-watcher-calendars.exe open-logs
./aw-watcher-calendars.exe about
```

`--headless` is an action-only validation/manual-report option: configure validates
the existing file rather than opening the editor; other actions print the same
privacy-safe report instead of displaying a dialog. It is not advertised in the
tray manifest and does not bypass action context validation. Native UI is Windows
only; no Unix GUI compatibility claim is made.

Connection reachability is not proof of a feed being accessible, latest sync
success, attendance, or producer liveness. Recent log entries can be old; their
timestamps are shown and no current-running state is inferred. A successful status
helper exit means the report was displayed, even if it reports AW unavailable.
Action completion is recorded separately in `aw-watcher-calendars-actions.log`
(testing: `aw-watcher-calendars-testing-actions.log`) under the same AW log folder.
Explicit config paths use a neighboring `.actions.log`. These rotating logs contain
only timestamps, fixed action IDs and completion booleans; no args/config/errors.

## Package, installation and rollback

Use `./scripts/build.ps1 -Test -Package`. It places the matching UTF-8 sidecar beside
the release executable and inside the ZIP. The existing installer copies the pair
into AW's module folder. Config and autostart behavior remain unchanged.

Before updating, exit AW and retain the previous executable **and matching
sidecar**, if present. Install using the existing one-line `install.ps1` command.
For rollback, restore the pair from the same version. Running the installer with
an older executable that has no adjacent `.actions.json` automatically removes
the installed stale sidecar. Manual rollback to a version without action commands
must likewise remove only `aw-watcher-calendars.exe.actions.json`. Do not delete local
configuration, logs, buckets or stored history. This task performed no deployment.

## Local validation and limits

`python tests/actions.py` uses isolated temporary config/profile folders and
headless helpers. It covers all manifest commands, unknown commands, protocol/ID
validation, AW_TESTING/AW_PROFILE isolation, server override precedence, original
CLI fallback, stopped server, valid synthetic server info, safe log filtering,
action dispatch while a real isolated producer owns its lock, duplicate producer
rejection, and release/ZIP manifest equality. Installer smoke tests check matching
sidecar copies, repeated installation and removal when the supplied executable
has no sidecar. The Rust unit test checks invalid-edit preservation and atomic Save.

The initial integration only compiled/linked native windows and ran headless
tests. A follow-up reproduced the host's actual hidden-console launch condition:
`STARTF_USESHOWWINDOW/SW_HIDE` made the editor invisible while its process stayed
alive. Normal shell launch did not reproduce it. The fixed editor consumes that
startup flag, explicitly shows the completed window, and owns Save/error dialogs.

With explicit permission, `python tests/native_actions.py -v` now opens and operates
real native windows on synthetic named/testing-profile data, using exactly the
manifest `configure` command and hidden-startup context. Three tests pass: initial
text/control visibility, Cancel preservation, invalid Save preservation, owned
modal error/confirmation dialogs, a >32 KiB valid Save, window-close preservation,
clean exits and no producer lock. The installed pre-fix binary fails the visibility
test; the fixed release and staged package pass. The test may terminate only its
own action helper. It is opt-in and must not be run without UI authorization.

No actual installed-tray click, live private-config action, live server action,
host-exit dialog cleanup or reboot was tested. Pixel-level visual signoff remains
unperformed: native visibility/content/control interaction assertions are the
evidence, not screenshot rendering. Host ownership/reaping is not established by
component tests. See [validation evidence](validation.md). No GitHub workflows
were added or used.

## Shared menu convention

Use Running first (owned by AW), then Settings…, Status…, Logs…, About… in that order. Omit unsupported actions. Stable action IDs and existing command arguments stay unchanged. Module-specific details belong inside the dialogs rather than common menu labels. Logs are privacy-safe; Status must distinguish server reachability, producer state and delivery evidence.
