#!/usr/bin/env python3
"""A session's view of the control plane: follow its events, brief it, show its status line.

Stdlib only; run as `python3 -I tooling/session/session.py <follow|brief|statusline|stop|install>`,
through `tooling/session/session.sh`, which prefers `pigeon session`.
See .claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md §2 and §3, and
.claude/decisions/2026-11-13-development-rust-session-event-follower.md §4.
"""

import hashlib
import json
import os
import subprocess
import sys
import time

HOST = os.environ.get("PIGEON_HOST", "platform.pigeon.dev")
SESSION = os.path.expanduser("~/.config/pigeon/session.json")
CACHE = os.path.expanduser("~/.cache/pigeon/status.json")
BIN = os.path.expanduser("~/.cache/pigeon/bin/pigeon")
WAIT = 20  # under the proxy's ~30 s cut; see 2026-11-13-development-rust-session-event-follower.md §1
STALE = 10 * 60
BRIEF = 10

START = """No pigeon session yet. Per CLAUDE.md:
1. `git remote add artifacts https://platform.pigeon.dev/git/pigeon/pigeon.git`; fetch its main and branch from it.
2. `session_start` with this session's URL, then write {"label": ..., "ticket": ...} to ~/.config/pigeon/session.json.
3. `change_propose` with the ticket before writing the change; follow with `sh tooling/session/session.sh follow`."""


def label():
    try:
        with open(SESSION) as f:
            return json.load(f).get("label")
    except (OSError, ValueError):
        return None


def get(path, timeout):
    """GETs `path` through the agent's proxy, which authenticates the host; None on failure."""
    out = subprocess.run(
        ["curl", "-sS", "--max-time", str(timeout), f"https://{HOST}{path}"],
        capture_output=True,
        text=True,
    ).stdout
    try:
        return json.loads(out)
    except ValueError:
        return None


def run(operation, args, timeout=30):
    """Runs `operation` on the control plane: its result, or None."""
    body = json.dumps({"operation": operation, "args": args})
    out = subprocess.run(
        ["curl", "-sS", "--max-time", str(timeout), "-X", "POST",
         "-H", "content-type: application/json", "--data", body, f"https://{HOST}/v0/run"],
        capture_output=True,
        text=True,
    ).stdout
    try:
        return json.loads(out).get("result")
    except (ValueError, AttributeError):
        return None


def status(me, timeout=30):
    """The session's status, fresh, saved for the status line; None when unreachable."""
    s = run("session.status", {"session": me}, timeout)
    if s is not None:
        s["at"] = int(time.time())
        os.makedirs(os.path.dirname(CACHE), exist_ok=True)
        with open(CACHE + ".tmp", "w") as f:
            json.dump(s, f)
        os.replace(CACHE + ".tmp", CACHE)
    return s


def cached():
    try:
        with open(CACHE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def quote(v):
    return "".join(c if c.isalnum() or c in "-._~" else "%%%02X" % ord(c) for c in v)


def follow():
    """Prints one line per event the agent must act on; resumes from the control plane's cursor."""
    me = label()
    if not me:
        print(START.splitlines()[0], flush=True)
        return 1
    after, bad = "session", 0
    while True:
        d = get(f"/v0/events?session={quote(me)}&after={after}&wake=agent&seconds={WAIT}", WAIT + 15)
        if d is None:
            bad += 1
            if bad == 5:
                print("events: 5 failed polls in a row; still trying", flush=True)
            time.sleep(min(60, 5 * bad))
            continue
        bad = 0
        for e in d.get("events", []):
            st = e.get("stage") or {}
            n = f"!{e['change']} " if e.get("change") else ""
            where = f"{st['stage']} needs={st['needs']} " if st else ""
            print(f"{e['id']} {n}{where}{e['kind']}: {e['summary']}", flush=True)
            after = e["id"]
        if after == "session":
            after = d.get("last", 0)
        status(me)


def line(s):
    parts = [f"!{c['number']} {c['stage']['stage']}" for c in s.get("changes", [])]
    ev = s.get("events") or {}
    parts.append("listening" if ev.get("listening") else "not listening")
    if time.time() - s.get("at", 0) > STALE:
        parts.append("stale")
    return " · ".join(parts)


def brief():
    """For the SessionStart hook: where this session is, from the control plane."""
    me = label()
    if not me:
        print(START)
        return 0
    s = status(me, BRIEF) or cached()
    if s is None:
        print(f"pigeon session {me}: the control plane didn't answer; `session_status` when it does.")
        return 0
    print(f"pigeon session {me}:")
    changes = s.get("changes", [])
    changes.sort(key=lambda c: c["stage"]["needs"] != "agent")
    for c in changes:
        st = c["stage"]
        print(f"- !{c['number']} {c['title']}: {st['stage']}, needs {st['needs']}: {st['next']}")
    if not changes:
        print("- no open changes")
    for t in s.get("tasks", []):
        print(f"- task {t['id']} ({t['kind']}) on !{t['change']}: {t['note']}")
    ev = s.get("events") or {}
    if not ev.get("listening"):
        print("- not following events: arm Monitor with `sh tooling/session/session.sh follow`")
    return 0


def statusline():
    """For the status line: the cache only, never the network."""
    sys.stdin.read() if not sys.stdin.isatty() else None
    s = cached()
    print(line(s) if s else "pigeon: no session status yet")
    return 0


def stop():
    """For the Stop hook: a reminder when a change needs this session and nothing follows events."""
    sys.stdin.read() if not sys.stdin.isatty() else None
    s = cached()
    if not s or (s.get("events") or {}).get("listening"):
        return 0
    waiting = [c for c in s.get("changes", []) if c["stage"]["needs"] == "agent"]
    if waiting:
        said = ", ".join(f"!{c['number']} ({c['stage']['next']})" for c in waiting)
        print(json.dumps({"systemMessage": f"pigeon: {said} need this session, and nothing follows its events."}))
    return 0


def install():
    """Fetches main's newest pigeon build to BIN, SHA-256 checked; nothing when it is there."""
    b = ((run("deploy.status", {}, 10) or {}).get("pipeline") or {}).get("latest_build") or {}
    sha, want = b.get("sha"), b.get("sha256")
    if not sha or not want:
        return 1
    try:
        with open(BIN + ".sha") as f:
            if f.read().strip() == sha and os.access(BIN, os.X_OK):
                return 0
    except OSError:
        pass
    os.makedirs(os.path.dirname(BIN), exist_ok=True)
    tmp = BIN + ".tmp"
    got = subprocess.run(["curl", "-sSf", "--max-time", "60", "-o", tmp, f"https://{HOST}/builds/{sha}/pigeon"])
    ok = got.returncode == 0
    if ok:
        with open(tmp, "rb") as f:
            ok = hashlib.sha256(f.read()).hexdigest() == want
    if not ok:
        if os.path.exists(tmp):
            os.remove(tmp)
        return 1
    os.chmod(tmp, 0o755)
    os.replace(tmp, BIN)
    with open(BIN + ".sha", "w") as f:
        f.write(sha)
    return 0


def main():
    commands = {"follow": follow, "brief": brief, "statusline": statusline, "stop": stop, "install": install}
    name = sys.argv[1] if len(sys.argv) > 1 else ""
    if name not in commands:
        print(f"usage: session.py {'|'.join(commands)}", file=sys.stderr)
        return 2
    try:
        return commands[name]()
    except KeyboardInterrupt:
        return 0
    except Exception:  # noqa: BLE001 - a hook never stops a session
        return 0 if name != "follow" else 1


if __name__ == "__main__":
    sys.exit(main())
