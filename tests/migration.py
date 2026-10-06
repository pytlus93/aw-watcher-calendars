import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("migration", Path(__file__).parents[1] / "scripts" / "migrate-config.py")
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


class MigrationTests(unittest.TestCase):
    def test_preserves_identity_source_settings_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            old = {"calendar_id": "existing-work", "calendar_url": "https://example.invalid/private.ics",
                   "calendar_file": None, "excluded_busy_statuses": ["OOF"], "reconcile": False, "poll_seconds": 3600}
            path.write_text(json.dumps(old), encoding="utf-8")
            self.assertTrue(migration.migrate(path))
            new = json.loads(path.read_text())
            self.assertEqual(new["calendars"], [{"calendar_id": "existing-work", "enabled": True,
                                               "calendar_url": "https://example.invalid/private.ics"}])
            self.assertEqual(new["excluded_busy_statuses"], ["OOF"])
            self.assertFalse(new["reconcile"])
            first = path.read_bytes()
            self.assertFalse(migration.migrate(path))
            self.assertEqual(path.read_bytes(), first)

    def test_invalid_source_preserves_original_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            path.write_text('{"calendar_file":"synthetic.ics","calendar_url":"https://example.invalid/also.ics"}')
            before = path.read_bytes()
            with self.assertRaises(ValueError):
                migration.migrate(path)
            self.assertEqual(path.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
