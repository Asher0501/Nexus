"""Nexus node: retro summary node."""
import sys
import json

ctx = json.load(sys.stdin)
output = {"route": "done", "content": "Workflow completed successfully"}
json.dump(output, sys.stdout)
