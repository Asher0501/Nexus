"""Nexus node: approved handler."""
import sys
import json

ctx = json.load(sys.stdin)
output = {"route": "complete", "content": "approved handler done"}
json.dump(output, sys.stdout)
