"""Nexus node: code reviewer (simulated)."""
import sys
import json

ctx = json.load(sys.stdin)
inputs = ctx.get("inputs", {})
run_count = ctx.get("metadata", {}).get("run_count", 1)

# Simulate review: first 2 runs → rejected, 3rd run → approved
if run_count >= 3:
    output = {"route": "approved", "content": "code looks good"}
else:
    output = {"route": "rejected", "content": f"issues found (run {run_count})"}
json.dump(output, sys.stdout)
