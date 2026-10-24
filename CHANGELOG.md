# Changelog

## 1.0.0 — 2026-10-24

- Rename the repository and release packages to `aw-watcher-calendars`.
- Rename the Cargo crate, executable, AW module/actions identity, settings/log paths
  and generated per-calendar bucket prefix to `aw-watcher-calendars`.
- Leave earlier installed files, private settings and recorded history untouched;
  migration is a separate explicit operation. UID/occurrence logic is unchanged.
- Add `--port <port>` for imports and management actions. It overrides the selected
  loopback origin's port, including testing and per-calendar saved endpoints; absent
  a CLI/action server origin, it uses 127.0.0.1. Profiles/testing remain explicit.
- Derive cleanly staged release ZIP names from Cargo metadata and add SHA256 sidecars.
- Retain existing import scopes, recurrence/OOF handling, privacy projection,
  reconciliation, migration and persistent bodyless-304 protocol regressions.

Calendar intervals describe scheduled context, not foreground work or attendance.
Existing development implemented the import features above; this release changes
naming, CLI port selection and packaging. No native UI or live feed validation is
implied by headless tests, and no automatic update source is registered.
