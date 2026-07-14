"""Write review and fix content to review.md and fix.md."""
import sys, json
ctx = json.load(sys.stdin)
inputs = ctx.get("inputs", {})
rev = inputs.get("review", "")
fix = inputs.get("fix", "")
with open("review.md", "w", encoding="utf-8") as f:
    f.write(rev)
with open("fix.md", "w", encoding="utf-8") as f:
    f.write(fix)
print(json.dumps({"route": "ok", "content": f"review.md ({len(rev)} chars) + fix.md ({len(fix)} chars) written"}))
