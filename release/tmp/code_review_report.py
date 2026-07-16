"""Save the code review report from review_doc.md."""
import sys, json, os, re
from datetime import datetime

if sys.platform == "win32":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

_SURROGATE_RE = re.compile(r'[\ud800-\udfff]')
def sanitize(s): return _SURROGATE_RE.sub('�', s)

def main():
    doc_path = "review_doc.md"
    if not os.path.exists(doc_path):
        doc_path = os.path.join("..", doc_path)

    content = ""
    if os.path.exists(doc_path):
        with open(doc_path, "r", encoding="utf-8") as f:
            content = f.read()
    if not content:
        content = "No review findings found."

    ts = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    report = (
        "# Nexus Engine — Code Review Report\n\n"
        f"**Generated**: {ts}\n"
        "**Status**: COMPLETED\n\n"
        "---\n\n"
        "## Findings & Fixes\n\n" + content
    )
    with open("CODE_REVIEW_REPORT.md", "w", encoding="utf-8") as f:
        f.write(report)
    print(json.dumps({
        "route": "report_saved",
        "content": sanitize(f"Report saved ({len(report)} chars)")
    }, ensure_ascii=False))

if __name__ == "__main__":
    main()
