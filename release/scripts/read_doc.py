"""Read the design philosophy document and output as Nexus node content."""
import sys, json
with open("../bak/theory/DESIGN_PHILOSOPHY.md", encoding="utf-8") as f:
    doc = f.read()
json.dump({"route": "ok", "content": doc}, sys.stdout)
