"""Nexus node: start node that provides seed data."""
import sys
import json

ctx = json.load(sys.stdin)
output = {
    "route": "ok",
    "content": "fn divide(a: i32, b: i32) -> i32 { a / b }"
}
json.dump(output, sys.stdout)
