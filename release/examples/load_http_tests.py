"""Load HTTP test workflows into Nexus Dashboard and run them.

Usage:
  python load_http_tests.py [--dashboard http://127.0.0.1:48080]
"""

import json, sys, urllib.request, os

DASHBOARD = os.environ.get("NEXUS_DASHBOARD", "http://127.0.0.1:48080")
EXAMPLES = os.path.dirname(os.path.abspath(__file__))


def post_json(url: str, data: dict) -> dict:
    body = json.dumps(data, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(url, data=body, method="POST")
    req.add_header("Content-Type", "application/json; charset=utf-8")
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read().decode("utf-8"))


def load_workflow(filepath: str) -> str | None:
    """Load .json workflow, POST to Dashboard, return workflow ID."""
    basename = os.path.basename(filepath)
    with open(filepath, encoding="utf-8") as f:
        definition = json.load(f)

    name = basename.replace(".json", "")
    try:
        result = post_json(f"{DASHBOARD}/api/workflows", {
            "name": f"[HTTP Test] {name}",
            "definition": definition,
        })
        wf_id = result.get("id")
        print(f"  ✅ {name} → id={wf_id}")
        return wf_id
    except Exception as e:
        print(f"  ❌ {name}: {e}")
        return None


def run_workflow(wf_id: str):
    """Trigger a workflow run."""
    try:
        result = post_json(f"{DASHBOARD}/api/workflows/{wf_id}/run", {})
        run_id = result.get("run_id", "?")
        print(f"     🚀 run_id={run_id}")
    except Exception as e:
        print(f"     ❌ run failed: {e}")


def main():
    workflows = [
        "http-test.json",
        "http-test-post.json",
        "http-test-branch.json",
    ]

    print(f"Dashboard: {DASHBOARD}")
    print(f"Loading {len(workflows)} HTTP test workflows...\n")

    ids = []
    for wf in workflows:
        path = os.path.join(EXAMPLES, wf)
        if os.path.exists(path):
            wf_id = load_workflow(path)
            if wf_id:
                ids.append(wf_id)
        else:
            print(f"  ⚠️  {wf} not found")

    if ids:
        print(f"\nRunning {len(ids)} workflows...")
        for wf_id in ids:
            run_workflow(wf_id)

    print("\nDone. Open Dashboard to see results:")
    print(f"  {DASHBOARD}")


if __name__ == "__main__":
    main()
