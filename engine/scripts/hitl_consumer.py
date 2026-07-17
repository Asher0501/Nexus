"""
HITL Consumer — terminal interaction process for the human pool.

Connects to the HTTP pool server, displays pending questions one at a
time, reads answers from the terminal (CONIN$/dev/tty), and posts them
back to the pool.

Usage:
  python hitl_consumer.py                # default port 19876
  python hitl_consumer.py --port 19877
"""

import json
import sys
import time
import urllib.request
import urllib.error

POOL_PORT = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[1] == "--port" else 19876
POOL_URL = f"http://127.0.0.1:{POOL_PORT}"


def open_tty():
    """Open the real terminal for input (bypasses stdin pipe)."""
    try:
        if sys.platform == "win32":
            return open("CONIN$", "r", encoding="utf-8", errors="replace")
        return open("/dev/tty", "r", encoding="utf-8", errors="replace")
    except (OSError, IOError):
        return None


def display_question(q):
    """Pretty-print a question to stderr (terminal)."""
    sys.stderr.write("\n" + "=" * 54 + "\n")
    if q.get("context"):
        sys.stderr.write(f"Context: {q['context']}\n")
    sys.stderr.write(f"Question: {q['question']}\n")
    opts = q.get("options", [])
    if opts:
        sys.stderr.write("Options:\n")
        for i, o in enumerate(opts):
            sys.stderr.write(f"  [{i + 1}] {o}\n")
        sys.stderr.write("Enter number or type answer: ")
    else:
        sys.stderr.write("Your answer: ")
    sys.stderr.flush()


def post_answer(qid, answer):
    body = json.dumps({"answer": answer}).encode()
    req = urllib.request.Request(f"{POOL_URL}/a/{qid}", data=body, method="POST")
    req.add_header("Content-Type", "application/json")
    urllib.request.urlopen(req)


def main():
    tty = open_tty()
    if not tty:
        sys.stderr.write("[consumer] no terminal available\n")
        return

    try:
        while True:
            # Check for pending questions
            try:
                resp = urllib.request.urlopen(f"{POOL_URL}/pending", timeout=10)
                data = json.loads(resp.read().decode())
                questions = data.get("questions", [])
            except urllib.error.URLError:
                time.sleep(1)
                continue
            except Exception:
                time.sleep(1)
                continue

            if not questions:
                time.sleep(0.5)
                continue

            # Display and answer each question
            for q in questions:
                display_question(q)
                line = tty.readline()
                if not line:
                    break
                answer = line.strip()
                if answer.isdigit() and q.get("options"):
                    idx = int(answer) - 1
                    opts = q["options"]
                    if 0 <= idx < len(opts):
                        answer = opts[idx]
                post_answer(q["qid"], answer)
                sys.stderr.write(f"Received: {answer}\n" + "=" * 54 + "\n")
                sys.stderr.flush()

    except KeyboardInterrupt:
        pass
    finally:
        try:
            tty.close()
        except Exception:
            pass


if __name__ == "__main__":
    main()
