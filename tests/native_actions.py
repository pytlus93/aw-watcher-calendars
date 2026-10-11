"""Opt-in Windows UI execution tests. Opens ONLY owned synthetic configuration windows.

Run with explicit user authorization: python tests/native_actions.py
--exe may select an installed binary for isolated reproduction (no deployment).
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

parser = argparse.ArgumentParser(add_help=False)
parser.add_argument("--exe", type=Path, default=Path(__file__).parents[1] / "target/release/aw-calendar-sync.exe")
options, remaining = parser.parse_known_args()
EXE = options.exe.resolve()

if os.name == "nt":
    user32 = c.WinDLL("user32", use_last_error=True)
    Callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    for name, arguments in {
        "EnumWindows": [Callback, w.LPARAM],
        "EnumChildWindows": [w.HWND, Callback, w.LPARAM],
        "GetWindowThreadProcessId": [w.HWND, c.POINTER(w.DWORD)],
        "GetClassNameW": [w.HWND, w.LPWSTR, c.c_int],
        "GetWindowTextW": [w.HWND, w.LPWSTR, c.c_int],
        "IsWindowVisible": [w.HWND],
        "IsWindowEnabled": [w.HWND],
        "GetWindow": [w.HWND, w.UINT],
        "PostMessageW": [w.HWND, w.UINT, w.WPARAM, w.LPARAM],
        "SendMessageW": [w.HWND, w.UINT, w.WPARAM, w.LPARAM],
    }.items():
        getattr(user32, name).argtypes = arguments
    user32.GetWindow.restype = w.HWND
    user32.SendMessageW.restype = w.LPARAM


@unittest.skipUnless(os.name == "nt", "Native Windows UI only")
class NativeConfigurationTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory(prefix="calendar-native-ui-")
        self.root = Path(self.folder.name)
        self.config = self.root / "activitywatch-synthetic-ui/activitywatch-synthetic-ui/aw-calendar-sync/config-testing.json"
        self.config.parent.mkdir(parents=True)
        self.original = json.dumps({"calendars": [{"calendar_id": "synthetic", "enabled": False}]})
        self.config.write_text(self.original)
        # Exactly the manifest command; config isolation comes from real protocol context.
        env = {**os.environ, "LOCALAPPDATA": str(self.root), "AW_MODULE_ACTION_PROTOCOL": "1",
               "AW_MODULE_ACTION": "configure", "AW_TESTING": "1", "AW_PROFILE": "synthetic-ui",
               "AW_SERVER_URL": "http://127.0.0.1:59999"}
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0  # Reproduce enhanced aw-qt, NOT a regular shell launch.
        self.child = subprocess.Popen([str(EXE), "configure"], env=env, startupinfo=startup,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def tearDown(self):
        if self.child.poll() is None:
            self.child.terminate()  # Only this test's direct helper, never a live module.
        self.child.communicate(timeout=5)
        self.folder.cleanup()

    def windows(self, parent=None):
        found = []
        @Callback
        def visit(handle, _):
            pid = w.DWORD()
            user32.GetWindowThreadProcessId(handle, c.byref(pid))
            if pid.value == self.child.pid:
                name, title = c.create_unicode_buffer(256), c.create_unicode_buffer(512)
                user32.GetClassNameW(handle, name, len(name))
                user32.GetWindowTextW(handle, title, len(title))
                found.append((handle, name.value, title.value))
            return True
        if parent is None:
            user32.EnumWindows(visit, 0)
        else:
            user32.EnumChildWindows(parent, visit, 0)
        return found

    def wait(self, predicate, description):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            if self.child.poll() is not None:
                self.fail(f"Helper exited before {description}; code={self.child.returncode}")
            time.sleep(.05)
        self.fail(f"Timed out waiting for {description}")

    def editor(self):
        window = self.wait(lambda: next((h for h, name, _ in self.windows() if name == "AwCalendarConfig"), None), "editor window")
        self.wait(lambda: user32.IsWindowVisible(window), "visible editor under SW_HIDE launch")
        controls = self.windows(window)
        for handle, name, title in controls:
            self.assertTrue(user32.IsWindowVisible(handle), f"Invisible {name} control")
            if name == "Edit":
                initial = c.create_unicode_buffer(len(self.original) + 1)
                user32.SendMessageW(handle, 0xd, len(initial), c.cast(initial, c.c_void_p).value)
                self.assertEqual(initial.value, self.original)
        self.assertEqual({title for _, name, title in controls if name == "Button"}, {"Save", "Cancel"})
        return window

    def click(self, parent, label):
        button = next(h for h, name, title in self.windows(parent) if name == "Button" and title == label)
        self.assertTrue(user32.PostMessageW(button, 0xf5, 0, 0))  # BM_CLICK; actual parent notification.

    def set_text(self, editor, text):
        edit = next(h for h, name, _ in self.windows(editor) if name == "Edit")
        buffer = c.create_unicode_buffer(text)
        self.assertEqual(user32.SendMessageW(edit, 0xc, 0, c.cast(buffer, c.c_void_p).value), 1)
        readback = c.create_unicode_buffer(len(text) + 1)
        user32.SendMessageW(edit, 0xd, len(readback), c.cast(readback, c.c_void_p).value)
        self.assertEqual(readback.value, text)

    def dialog(self, editor, expected):
        dialog = self.wait(lambda: next((h for h, name, _ in self.windows() if name == "#32770"), None), "validation/Save dialog")
        self.assertTrue(user32.IsWindowVisible(dialog))
        self.assertEqual(user32.GetWindow(dialog, 4), editor)  # GW_OWNER
        self.assertFalse(user32.IsWindowEnabled(editor))
        message = " ".join(title for _, name, title in self.windows(dialog) if name == "Static")
        self.assertIn(expected, message)
        self.click(dialog, "OK")
        return dialog

    def assert_clean_exit(self):
        self.assertEqual(self.child.wait(timeout=5), 0)
        self.assertFalse(self.config.with_suffix(".lock").exists())
        output, errors = self.child.communicate(timeout=5)
        self.assertEqual(output, b"")
        self.assertEqual(errors, b"")

    def test_visible_editor_cancel_preserves_configuration(self):
        editor = self.editor()
        self.set_text(editor, "{invalid unsaved synthetic edit}")
        self.click(editor, "Cancel")
        self.assert_clean_exit()
        self.assertEqual(self.config.read_text(), self.original)

    def test_validation_error_then_large_valid_save_through_native_controls(self):
        editor = self.editor()
        self.set_text(editor, "{invalid synthetic edit}")
        self.click(editor, "Save")
        dialog = self.dialog(editor, "Not saved:")
        self.assertEqual(self.config.read_text(), self.original)
        self.wait(lambda: all(h != dialog for h, _, _ in self.windows()), "dismissed error dialog")
        self.wait(lambda: user32.IsWindowEnabled(editor), "enabled editor after error")
        replacement = json.dumps({"calendars": [{"calendar_id": "synthetic", "enabled": False}],
                                  "include_summary": False}) + " " * 40000
        self.set_text(editor, replacement)
        self.click(editor, "Save")
        self.dialog(editor, "Saved.")
        self.assert_clean_exit()
        self.assertEqual(self.config.read_text(), replacement)
        self.assertFalse(json.loads(self.config.read_text())["include_summary"])

    def test_window_close_preserves_configuration(self):
        editor = self.editor()
        self.assertTrue(user32.PostMessageW(editor, 0x10, 0, 0))
        self.assert_clean_exit()
        self.assertEqual(self.config.read_text(), self.original)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0], *remaining])
