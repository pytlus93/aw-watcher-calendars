"""Headless isolated protocol/packaging tests; no native dialog or live watcher opened."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import threading

ROOT = Path(__file__).parents[1]
EXE = ROOT / "target/release/aw-watcher-calendars.exe"


class ActionsTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory(prefix="calendar-actions-")
        self.root = Path(self.folder.name)
        self.config = self.root / "config.json"
        self.config.write_text(json.dumps({"calendars": [{"calendar_id": "synthetic", "enabled": False}]}))
        self.env = os.environ.copy()
        for key in ["AW_MODULE_ACTION_PROTOCOL", "AW_MODULE_ACTION", "AW_TESTING", "AW_PROFILE", "AW_SERVER_URL"]:
            self.env.pop(key, None)
        self.env["LOCALAPPDATA"] = str(self.root / "appdata")

    def tearDown(self):
        self.folder.cleanup()

    def run_action(self, command, extra=(), env=None, explicit=True):
        args = [str(EXE), command, "--headless", *extra]
        if explicit:
            args += ["--config", str(self.config)]
        return subprocess.run(args, env=env or self.env, capture_output=True, text=True, timeout=8)

    def test_dispatch_unknown_protocol_identity_and_no_collection(self):
        for command in ["configure", "open-logs", "about"]:
            result = self.run_action(command)
            self.assertEqual(result.returncode, 0, result.stderr)
        for command in ["refresh", "unknown"]:
            self.assertNotEqual(self.run_action(command).returncode, 0)
        for protocol, action in [("2", "about"), ("1", "wrong")]:
            env = {**self.env, "AW_MODULE_ACTION_PROTOCOL": protocol, "AW_MODULE_ACTION": action, "AW_TESTING": "0"}
            self.assertNotEqual(self.run_action("about", env=env).returncode, 0)
        self.assertFalse(self.config.with_suffix(".lock").exists())
        self.assertFalse(self.config.with_suffix(".log").exists())

    def test_protocol_profile_testing_and_manual_compatibility(self):
        env = {**self.env, "AW_MODULE_ACTION_PROTOCOL": "1", "AW_MODULE_ACTION": "configure",
               "AW_TESTING": "1", "AW_PROFILE": "synthetic-test", "AW_SERVER_URL": "http://127.0.0.1:59999"}
        result = self.run_action("configure", env=env, explicit=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        expected = self.root / "appdata/activitywatch-synthetic-test/activitywatch-synthetic-test/aw-watcher-calendars/config-testing.json"
        self.assertTrue(expected.exists())
        self.assertIn(str(expected), result.stdout)
        manual = {**self.env, "AW_PROFILE": "synthetic-test"}
        result = self.run_action("configure", extra=("--testing",), env=manual, explicit=False)
        self.assertEqual(result.returncode, 0)
        self.assertIn(str(expected), result.stdout)
        self.assertEqual(len(list(self.root.rglob("config-testing.json"))), 1)
        bad = {**env, "AW_SERVER_URL": "https://remote.example.invalid"}
        self.assertNotEqual(self.run_action("configure", env=bad).returncode, 0)
        self.assertEqual(self.run_action("configure", extra=("--server-url", "http://127.0.0.1:59999"), env=bad).returncode, 0)

    def test_actions_bypass_running_producer_singleton(self):
        child = subprocess.Popen([str(EXE), "--config", str(self.config)], env=self.env,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 5
            while not self.config.with_suffix(".lock").exists() and time.monotonic() < deadline:
                time.sleep(.05)
            self.assertIsNone(child.poll())
            for command in ["configure", "open-logs", "about"]:
                self.assertEqual(self.run_action(command).returncode, 0)
                self.assertIsNone(child.poll())
            duplicate = subprocess.run([str(EXE), "--config", str(self.config), "--once"],
                                       env=self.env, capture_output=True, timeout=5)
            self.assertNotEqual(duplicate.returncode, 0)
            self.assertIn(b"another instance", duplicate.stderr)
        finally:
            child.terminate()
            child.wait(timeout=5)

    def test_logs_exclude_raw_private_values(self):
        self.config.with_suffix(".log").write_text("2026-10-11T10:00:00Z calendar=private-id eligible=2 inserted=1 unchanged=1 changed=0 deleted=0 secret-topic\n"
                                                   "2026-10-11T10:01:00Z private https://example.invalid/secret attendee@example.invalid\n")
        result = self.run_action("open-logs")
        self.assertEqual(result.returncode, 0)
        self.assertIn("inserted=1", result.stdout)
        for secret in ["private-id", "secret-topic", "https://", "attendee@example.invalid"]:
            self.assertNotIn(secret, result.stdout + result.stderr)

    def test_invalid_ports_are_rejected_without_ui(self):
        for port in ["0", "65536", "bad", "-1"]:
            self.assertNotEqual(self.run_action("about", extra=("--port", port)).returncode, 0)

    def test_status_consumes_server_context_and_reports_unavailability(self):
        requests = []
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self):
                requests.append(self.path)
                body = b'{"hostname":"synthetic","version":"synthetic"}'
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        env = {**self.env, "AW_MODULE_ACTION_PROTOCOL": "1", "AW_MODULE_ACTION": "status", "AW_TESTING": "1",
               "AW_SERVER_URL": f"http://127.0.0.1:{server.server_port}"}
        try:
            result = self.run_action("status-ui", env=env)
            self.assertEqual(result.returncode, 0)
            self.assertIn("reachable; valid info response", result.stdout)
            self.assertEqual(requests, ["/api/0/info"])
            via_port = self.run_action("status-ui", extra=("--port", str(server.server_port)),
                                       env={**env, "AW_SERVER_URL": "http://127.0.0.1:1"})
            self.assertEqual(via_port.returncode, 0, via_port.stderr)
            self.assertIn("reachable; valid info response", via_port.stdout)
            self.assertEqual(requests, ["/api/0/info", "/api/0/info"])
            self.config.write_text(json.dumps({"server_url": env["AW_SERVER_URL"],
                "calendars": [{"calendar_id": "synthetic", "enabled": False}]}))
            manual = self.run_action("status-ui")
            self.assertIn("reachable; valid info response", manual.stdout)
            self.assertEqual(requests, ["/api/0/info", "/api/0/info", "/api/0/info"])
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=5)
        result = self.run_action("status-ui", env=env)
        self.assertEqual(result.returncode, 0)
        self.assertIn("unavailable or invalid info response", result.stdout)
        self.assertIn("sync success were not tested", result.stdout)
        self.assertFalse(self.config.with_suffix(".lock").exists())

    def test_manifest_build_and_zip_match_executable_commands(self):
        manifest_path = ROOT / "aw-watcher-calendars.exe.actions.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(manifest["schema_version"], 1)
        self.assertEqual(manifest["module"], "aw-watcher-calendars")
        self.assertEqual(len(manifest["actions"]), 4)
        self.assertEqual(manifest_path.read_bytes(), Path(str(EXE) + ".actions.json").read_bytes())
        for action in manifest["actions"]:
            env = {**self.env, "AW_MODULE_ACTION_PROTOCOL": "1", "AW_MODULE_ACTION": action["id"],
                   "AW_TESTING": "0", "AW_SERVER_URL": "http://127.0.0.1:59999"}
            self.assertEqual(self.run_action(action["args"][0], env=env).returncode, 0)
        with zipfile.ZipFile(ROOT / "dist/aw-watcher-calendars-1.0.0-windows-x64.zip") as archive:
            self.assertEqual(archive.read("aw-watcher-calendars/aw-watcher-calendars.exe.actions.json"), manifest_path.read_bytes())


if __name__ == "__main__":
    unittest.main()
