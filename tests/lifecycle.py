"""Isolated executable lifecycle/late-server validation; never touches production AW."""
import datetime as dt
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import unittest

EXE = Path(__file__).parents[1] / "target/release/aw-calendar-sync.exe"


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory(prefix="aw-calendar-lifecycle-")
        self.root = Path(self.folder.name)
        self.config = self.root / "config.json"
        self.config.write_text(json.dumps({"calendars": [
            {"calendar_id": "synthetic", "enabled": False}]}))
        self.children = []

    def tearDown(self):
        for child in self.children:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)
        self.folder.cleanup()

    def launch(self):
        child = subprocess.Popen([str(EXE), "--config", str(self.config)],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.children.append(child)
        return child

    def wait_log(self, text, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            path = self.config.with_suffix(".log")
            if path.exists() and text in path.read_text():
                return
            time.sleep(0.05)
        self.fail("Expected isolated module log did not appear")

    def test_duplicate_rejection_crash_relaunch_and_explicit_stop(self):
        first = self.launch()
        self.wait_log("no calendars enabled")
        duplicate = subprocess.run([str(EXE), "--config", str(self.config), "--once"],
                                   capture_output=True, timeout=5)
        self.assertNotEqual(duplicate.returncode, 0)
        self.assertIn(b"another instance", duplicate.stderr)
        first.kill()  # ONLY the child created by this isolated test.
        first.wait(timeout=5)
        second = self.launch()
        time.sleep(0.2)
        self.assertIsNone(second.poll())
        second.terminate()
        second.wait(timeout=5)
        time.sleep(0.2)
        self.assertIsNotNone(second.poll())
        # Relaunch is the host's responsibility; the module never self-spawns.

    def test_server_starts_late_running_module_retries_and_preserves_gap(self):
        with socket.socket() as reserve:
            reserve.bind(("127.0.0.1", 0))
            port = reserve.getsockname()[1]
        now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
        starts = [now - dt.timedelta(hours=4), now - dt.timedelta(hours=2)]
        feed = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"
        for i, start in enumerate(starts):
            end = start + dt.timedelta(minutes=30)
            feed += (f"BEGIN:VEVENT\r\nUID:synthetic-{i}\r\nSUMMARY:Synthetic meeting {i}\r\n"
                     f"DTSTART:{start:%Y%m%dT%H%M%SZ}\r\nDTEND:{end:%Y%m%dT%H%M%SZ}\r\nEND:VEVENT\r\n")
        (self.root / "calendar.ics").write_text(feed + "END:VCALENDAR\r\n")
        self.config.write_text(json.dumps({"server_url": f"http://127.0.0.1:{port}",
            "poll_seconds": 3600, "payload_mode": "strict", "calendars": [
                {"calendar_id": "synthetic", "calendar_file": "calendar.ics"}]}))
        events = []
        raw_bodies = []
        inserted = threading.Event()

        class Handler(BaseHTTPRequestHandler):
            bucket = False
            def log_message(self, *args):
                pass
            def reply(self, code, value):
                body = json.dumps(value).encode()
                self.send_response(code)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def do_GET(self):
                if "/events" in self.path:
                    self.reply(200, events)
                elif Handler.bucket:
                    self.reply(200, {"client": "aw-calendar-sync", "type": "calendar"})
                else:
                    self.reply(404, {})
            def do_POST(self):
                body = self.rfile.read(int(self.headers["Content-Length"]))
                if "/events" in self.path:
                    raw_bodies.append(body.decode())
                    for event in json.loads(body):
                        events.append({**event, "id": len(events) + 1})
                    self.reply(200, len(json.loads(body)))
                    if len(events) == 2:
                        inserted.set()
                else:
                    Handler.bucket = True
                    self.reply(200, {})

        child = self.launch()
        self.wait_log("connection failed")
        server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            self.assertTrue(inserted.wait(75), "Running module did not recover delivery")
            self.wait_log("inserted=2")
            self.assertIsNone(child.poll())
            self.assertEqual(len(events), 2)
            for expected, event, raw in zip(starts, events, raw_bodies):
                self.assertEqual(event["timestamp"], expected.isoformat().replace("+00:00", "Z"))
                self.assertEqual(event["duration"], 1800)
                self.assertNotIn("all_day", event["data"])
                self.assertNotIn("status", event["data"])
                self.assertLess(raw.index('"schema_version"'), raw.index('"title"'))
            self.assertEqual((starts[1] - starts[0]).total_seconds(), 7200)
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=5)


if __name__ == "__main__":
    unittest.main()
