"""Read architecture doc and output as Nexus node content."""
import sys, json
with open("../bak/design/ARCHITECTURE.md", encoding="utf-8") as f:
    doc = f.read()
json.dump({"route": "ok", "content": doc}, sys.stdout)
