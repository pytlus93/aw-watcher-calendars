# Moving the single existing installation to the new name

Version 1.0.0 fully renames the development module from `aw-calendar-sync` to
`aw-watcher-calendars`, including Cargo crate, executable/actions sidecar, AW module
ID, default settings/log directories, bucket client and generated bucket prefix.
No live installation or data is changed by building this release.

Before deploying:

1. Stop the old module through ActivityWatch and remove its old autostart entry.
2. Back up its private config and AW history. The old default config is under
   `%LOCALAPPDATA%/activitywatch/activitywatch/aw-calendar-sync/config.json`;
   the corresponding new directory ends in `aw-watcher-calendars`.
3. Install the new package with `-NoAutostart`, then transfer/review your private
   config at the new path. Do not commit feed URLs or attendee/context data.
4. Confirm source IDs, feed completeness, privacy choices, import scope and local
   server endpoint before starting the new module through AW.
5. Enable only the new module for subsequent automatic starts.

Generated buckets now start with `aw-watcher-calendars_`. Existing buckets retain
their old names and events; they are not renamed, deleted, imported or merged.
The new namespace can import the same scheduled events as historical buckets, so
review any cross-bucket aggregation before combining old/new calendar context.
Moving or deduplicating historical data requires a separately reviewed operation
with backups; the installer does not attempt it. User-supplied calendar IDs and
UID/original-occurrence logic are retained.
