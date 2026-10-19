# Calendar module guidance

Follow the latest parent AGENTS.md. This checkout is the AW-managed calendar
interval importer, not a foreground watcher or standalone service. Do not add
heartbeat/idle semantics or merge gaps between scheduled events.

`import_scope` defaults to completed; started/current_day explicitly permit full
scheduled ongoing/future intervals. Apply that policy consistently to parsing,
reconciliation and transport clock guards. Current-day boundaries are local and
exclusive at next midnight; backwards scopes retain up to seven days of catch-up.

Use `docs/data-contract.md` and `docs/validation.md` for custom schema, privacy,
migration and evidence boundaries. Preserve UID/original occurrence identity and
source/bucket isolation. Validate an entire feed before writes. Confirm replacements
before deletion; retained older history and removed/disabled sources stay frozen.

Use synthetic fixtures. Never print or track live ICS URLs, calendar titles,
attendees, local configs or exports. Logs must contain only source IDs/counts and
redacted errors. Disabled optional data must be absent from outgoing payloads.

Run `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`,
`./scripts/build.ps1 -Package`, `python tests/migration.py`,
`python tests/lifecycle.py` (about one minute), and `./tests/install-smoke.ps1`.
Use `python tests/transport.py` after packaging to verify persistent bodyless
creation 304 with nonzero Content-Length, producer restart and actual interval
delivery. Never infer this compatibility from the info endpoint alone.
The lifecycle harness may kill only child processes it created. Never install
over or stop live modules for validation without separate user authorization.

AW owns launch/recovery/UI; audit the host separately and distinguish host-source
tests from installed-host evidence. Do not claim login/reboot recovery from an
autostart config. Run `python tests/actions.py` after packaging for isolated tray
protocol checks; do not open native dialogs during routine automated tests.
The real native execution regression `python tests/native_actions.py -v` is opt-in
and requires explicit user authorization to open synthetic UI; it may close only
its own helpers and must never use a live private config. Package/install
the executable's exact `.actions.json` sidecar and remove it on incompatible rollback.
Dedicated calendar settings selectors and official-only strict payload remain
open requirements; do not disguise custom identity properties as AW standards.
