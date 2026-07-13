"""Nexus node: reviewer that approves."""
import sys
import json

# Read Nexus context from stdin
ctx = json.load(sys.stdin)
inputs = ctx.get("inputs", {})

# Simulate review — always approved for this example
output = {"route": "approved", "content": "review passed"}
json.dump(output, sys.stdout)
