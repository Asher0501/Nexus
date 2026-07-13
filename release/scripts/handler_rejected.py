"""Nexus node: rejected handler."""
import sys
import json

ctx = json.load(sys.stdin)
output = {"route": "complete", "content": "rejected handler done"}
json.dump(output, sys.stdout)
