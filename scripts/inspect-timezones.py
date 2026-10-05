"""Inspect a supplied local ICS without displaying meeting content."""
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8-sig")
text = re.sub(r"\r?\n[ \t]", "", text)
zones = sorted(set(re.findall(r'TZID[=:]("[^"]+"|[^;\r\n:]+)', text)))
print("Timezone identifiers:", zones)
print("Events:", text.count("BEGIN:VEVENT"))
for field in ("ATTENDEE", "DESCRIPTION", "LOCATION"):
    print(field + " present:", bool(re.search(r"^" + field + r"[;:]", text, re.M)))
