"""Move an existing single source into calendars without revealing its private URL."""
import json
import os
import sys
import tempfile
from pathlib import Path


def migrate(path):
    config = json.loads(path.read_text(encoding="utf-8-sig"))
    if "calendars" in config:
        return False
    source = {"calendar_id": config.pop("calendar_id", "outlook-work"), "enabled": True}
    for key in ("calendar_url", "calendar_file"):
        value = config.pop(key, None)
        if value is not None:
            source[key] = value
    if ("calendar_url" in source) == ("calendar_file" in source):
        raise ValueError("Expected exactly one configured calendar source")
    config["calendars"] = [source]
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent,
                                         prefix=path.name + ".", suffix=".tmp", delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(config, stream, indent=2, ensure_ascii=False)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()
    return True


if __name__ == "__main__":
    try:
        changed = migrate(Path(sys.argv[1]))
        print("Migrated existing calendar into calendars list." if changed else "Calendar list already configured; unchanged.")
    except Exception:
        # JSON error text can include confidential values; keep diagnostics redacted.
        print("Config migration failed; original file preserved.", file=sys.stderr)
        sys.exit(1)
