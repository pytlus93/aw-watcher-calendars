"""Real CLI import/replay against persistent bodyless 304 responses; no live AW."""
import datetime as dt
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest

EXE = Path(__file__).parents[1] / "target/release/aw-watcher-calendars.exe"


class TransportTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory(prefix="calendar-transport-")
        root = Path(self.folder.name)
        self.config = root / "config.json"
        start = dt.datetime.now(dt.timezone.utc).replace(microsecond=0) - dt.timedelta(hours=2)
        end = start + dt.timedelta(minutes=30)
        self.timestamp = start.isoformat().replace("+00:00", "Z")
        (root / "calendar.ics").write_text(f"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:synthetic\r\nSUMMARY:Synthetic\r\nDTSTART:{start:%Y%m%dT%H%M%SZ}\r\nDTEND:{end:%Y%m%dT%H%M%SZ}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n")
        state = self.state = {"race": True, "events": [], "creates": 0, "bad_post": False, "wrong_type": False, "peers": []}
        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            def handle(self):
                try:
                    super().handle()
                except ConnectionResetError:
                    pass  # Expected when a short-lived producer closes its pooled socket.
            def log_message(self, *args): pass
            def reply(self, code, data=None):
                self.send_response(code)
                if code == 304:
                    # A selected representation length, NOT bytes to buffer.
                    self.send_header("Content-Length", "67")
                    self.send_header("Connection", "keep-alive")
                    self.end_headers()
                    self.wfile.flush()
                    return
                body = json.dumps(data).encode()
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def do_GET(self):
                if "/events" in self.path:
                    self.reply(200, state["events"])
                elif state["race"]:
                    state["race"] = False
                    self.reply(404, {})
                else:
                    self.reply(200, {"client": "aw-watcher-calendars", "type": "afkstatus" if state["wrong_type"] else "calendar"})
            def do_POST(self):
                body = self.rfile.read(int(self.headers["Content-Length"]))
                if "/events" in self.path:
                    if state["bad_post"]:
                        self.reply(304)
                    else:
                        for event in json.loads(body):
                            state["events"].append({**event, "id": len(state["events"]) + 1})
                        self.reply(200, 1)
                else:
                    state["creates"] += 1
                    state["peers"].append(self.client_address)
                    self.reply(304)
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.worker = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.worker.start()
        self.config.write_text(json.dumps({"calendar_id": "synthetic", "calendar_file": "calendar.ics",
            "server_url": f"http://127.0.0.1:{self.server.server_port}"}))

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.worker.join(timeout=5)
        self.folder.cleanup()

    def run_import(self, extra=()):
        env = {k:v for k,v in os.environ.items() if k not in ["AW_MODULE_ACTION_PROTOCOL", "AW_MODULE_ACTION", "AW_TESTING", "AW_SERVER_URL"]}
        return subprocess.run([str(EXE), "--config", str(self.config), "--once", *extra], env=env,
                              capture_output=True, timeout=5)

    def test_explicit_port_overrides_testing_and_server_url_for_actual_import(self):
        result = self.run_import(("--testing", "--server-url", "http://127.0.0.1:1",
                                  "--port", str(self.server.server_port)))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.state["events"]), 1)

    def test_bodyless_creation_304_with_length_67_restarts_and_delivers(self):
        self.assertEqual(self.run_import().returncode, 0)
        self.state["race"] = True
        self.assertEqual(self.run_import().returncode, 0)
        self.assertEqual(self.state["creates"], 2)
        self.assertEqual(len(self.state["events"]), 1)
        self.assertEqual(self.state["events"][0]["timestamp"], self.timestamp)
        self.assertEqual(self.state["events"][0]["duration"], 1800)

    def test_event_post_304_and_incompatible_metadata_fail(self):
        self.state["bad_post"] = True
        self.assertNotEqual(self.run_import().returncode, 0)
        self.assertEqual(self.state["events"], [])
        self.state.update(bad_post=False, wrong_type=True, race=True)
        self.assertNotEqual(self.run_import().returncode, 0)
        self.assertEqual(self.state["events"], [])
        self.state.update(wrong_type=False, race=True)
        self.assertEqual(self.run_import().returncode, 0)
        self.assertEqual(len(self.state["events"]), 1)


if __name__ == "__main__":
    unittest.main()
