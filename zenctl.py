#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "supervisor>=4.2",
#   "httpx>=0.27",
#   "python-dotenv>=1.0",
# ]
# ///
"""
zenctl.py — manage the zen_proxy sidecar via supervisord.

Usage:
  ./zenctl.py start          — start proxy (supervisor daemon + zen_proxy program)
  ./zenctl.py stop           — stop proxy and supervisor
  ./zenctl.py restart        — restart zen_proxy program
  ./zenctl.py status         — show process status + health endpoint
  ./zenctl.py logs [-n N] [-f] — tail the proxy log (default last 40 lines; -f to follow)
  ./zenctl.py models         — list models available via proxy
  ./zenctl.py test [MODEL]   — send "tell me a joke" to MODEL (default claude-haiku-4-5)
  ./zenctl.py regen-models   — refresh ~/.codex/zen_models.json from live Zen API
"""

import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
from pathlib import Path

import httpx
from dotenv import load_dotenv

# ── env ───────────────────────────────────────────────────────────────────────
_REPO = Path(__file__).parent.resolve()
_ENV_PATH = _REPO / ".env"
if _ENV_PATH.exists():
    load_dotenv(_ENV_PATH)

PORT = int(os.environ.get("ZEN_PROXY_PORT", "9099"))
API_KEY = os.environ.get("OPENCODE_API_KEY", "")
PROXY_URL = f"http://127.0.0.1:{PORT}"

# supervisord / supervisorctl sockets live in /tmp so they need no install
_SOCK = Path("/tmp/zen_supervisor.sock")
_PID  = Path("/tmp/zen_supervisor.pid")
_LOG  = Path("/tmp/zen_proxy.log")
_SLOG = Path("/tmp/zen_supervisord.log")
_CFG  = Path("/tmp/zen_supervisord.conf")


# ── supervisor config (written fresh each start) ──────────────────────────────
def _write_supervisor_conf() -> Path:
    proxy_script = str(_REPO / "zen_proxy.py")
    conf = textwrap.dedent(f"""\
        [unix_http_server]
        file={_SOCK}
        chmod=0700

        [supervisord]
        pidfile={_PID}
        logfile={_SLOG}
        logfile_maxbytes=1MB
        nodaemon=false

        [rpcinterface:supervisor]
        supervisor.rpcinterface_factory = supervisor.rpcinterface:make_main_rpcinterface

        [supervisorctl]
        serverurl=unix://{_SOCK}

        [program:zen_proxy]
        command=uv run --script {proxy_script}
        autostart=true
        autorestart=true
        startretries=5
        stdout_logfile={_LOG}
        stderr_logfile={_LOG}
        stdout_logfile_maxbytes=2MB
        redirect_stderr=true
        environment=OPENCODE_API_KEY="{API_KEY}",ZEN_PROXY_PORT="{PORT}"
    """)
    _CFG.write_text(conf)
    return _CFG


# ── supervisor helpers ────────────────────────────────────────────────────────
def _supervisord_running() -> bool:
    if not _PID.exists():
        return False
    try:
        pid = int(_PID.read_text().strip())
        os.kill(pid, 0)
        return True
    except (ValueError, ProcessLookupError, PermissionError):
        return False


def _supervisorctl(*args: str) -> str:
    result = subprocess.run(
        [sys.executable, "-m", "supervisor.supervisorctl", "-c", str(_CFG), *args],
        capture_output=True, text=True,
    )
    return (result.stdout + result.stderr).strip()


def _health() -> dict:
    try:
        r = httpx.get(f"{PROXY_URL}/health", timeout=3)
        return r.json()
    except Exception:
        return {"status": "unreachable"}


def _response_json(r: httpx.Response, context: str) -> dict:
    try:
        return r.json()
    except ValueError:
        body = r.text.strip() or "<empty body>"
        print(
            f"ERROR: {context} returned non-JSON (status={r.status_code}, content-type={r.headers.get('content-type', 'unknown')})",
            file=sys.stderr,
        )
        print(body, file=sys.stderr)
        sys.exit(1)


# ── commands ──────────────────────────────────────────────────────────────────
def cmd_start(_args):
    if _supervisord_running():
        print(f"supervisord already running (pid={_PID.read_text().strip()})")
        return
    _write_supervisor_conf()
    subprocess.run([sys.executable, "-m", "supervisor.supervisord", "-c", str(_CFG)], check=True)
    # wait up to 5s for health
    for _ in range(10):
        time.sleep(0.5)
        h = _health()
        if h.get("status") == "ok":
            print(f"zen_proxy started — {h}")
            return
    print(f"zen_proxy started but health check unreachable — check {_LOG}")


def cmd_stop(_args):
    if not _supervisord_running():
        print("supervisord not running")
        return
    _supervisorctl("stop", "zen_proxy")
    try:
        pid = int(_PID.read_text().strip())
        os.kill(pid, signal.SIGTERM)
        for _ in range(10):
            time.sleep(0.5)
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
    except (ValueError, FileNotFoundError):
        pass
    print("zen_proxy stopped")


def cmd_restart(_args):
    if not _supervisord_running():
        print("supervisord not running — use 'start' first")
        sys.exit(1)
    out = _supervisorctl("restart", "zen_proxy")
    print(out)
    time.sleep(1)
    print(f"health: {_health()}")


def cmd_status(_args):
    if not _supervisord_running():
        print("zen_proxy STOPPED (supervisord not running)")
        return
    out = _supervisorctl("status", "zen_proxy")
    print(out)
    print(f"health: {_health()}")


def cmd_logs(args):
    n = getattr(args, "n", 40)
    follow = getattr(args, "f", False)
    if not _LOG.exists():
        print(f"No log file yet at {_LOG}")
        return
    if follow:
        # print the last n lines then stream new ones with `tail -f`
        subprocess.run(["tail", f"-{n}", "-f", str(_LOG)])
    else:
        lines = _LOG.read_text(errors="replace").splitlines()
        print("\n".join(lines[-n:]))


def cmd_models(_args):
    if not API_KEY:
        print("ERROR: OPENCODE_API_KEY not set", file=sys.stderr)
        sys.exit(1)
    r = httpx.get(
        f"{PROXY_URL}/v1/models",
        headers={"Authorization": f"Bearer {API_KEY}"},
        timeout=10,
    )
    data = _response_json(r, "GET /v1/models")
    ids = [m["id"] for m in data.get("data", [])]
    for mid in ids:
        print(mid)


def cmd_test(args):
    model = getattr(args, "model", None) or "claude-haiku-4-5"
    if not API_KEY:
        print("ERROR: OPENCODE_API_KEY not set", file=sys.stderr)
        sys.exit(1)
    print(f"=== {model} / tell me a joke ===")
    payload = {
        "model": model,
        "stream": False,
        "input": [{"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": "tell me a joke"}]}],
    }
    r = httpx.post(
        f"{PROXY_URL}/v1/responses",
        headers={"Authorization": f"Bearer {API_KEY}", "Content-Type": "application/json"},
        json=payload,
        timeout=30,
    )
    data = _response_json(r, "POST /v1/responses")
    if data.get("error"):
        print(f"ERROR: {data['error']}", file=sys.stderr)
        sys.exit(1)
    try:
        print(data["output"][0]["content"][0]["text"])
    except (KeyError, IndexError):
        print(json.dumps(data, indent=2))


def cmd_regen_models(_args):
    """Refresh ~/.codex/zen_models.json from the live Zen API."""
    if not API_KEY:
        print("ERROR: OPENCODE_API_KEY not set", file=sys.stderr)
        sys.exit(1)
    catalog_script = _REPO / "zen_models_to_catalog.py"
    out_path = Path.home() / ".codex" / "zen_models.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    print("Fetching models from OpenCode Zen…")
    r = httpx.get(
        "https://opencode.ai/zen/v1/models",
        headers={"Authorization": f"Bearer {API_KEY}"},
        timeout=15,
    )
    r.raise_for_status()
    models_json = r.text
    count = len(_response_json(r, "GET https://opencode.ai/zen/v1/models").get("data", []))
    print(f"Found {count} models. Generating catalog…")

    result = subprocess.run(
        [sys.executable, str(catalog_script)],
        input=models_json, capture_output=True, text=True, check=True,
    )
    out_path.write_text(result.stdout)
    written = len(json.loads(result.stdout)["models"])
    print(f"Written {written} entries → {out_path}")
    print(f"\nActivate in ~/.codex/config.toml:\n  model_catalog_json = \"{out_path}\"")


# ── main ──────────────────────────────────────────────────────────────────────
def main():
    p = argparse.ArgumentParser(prog="zenctl", description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    sub.add_parser("start",   help="Start proxy via supervisord")
    sub.add_parser("stop",    help="Stop proxy and supervisord")
    sub.add_parser("restart", help="Restart zen_proxy program")
    sub.add_parser("status",  help="Show process status + health")

    logs_p = sub.add_parser("logs", help="Show proxy log tail")
    logs_p.add_argument("-n", type=int, default=40, help="Number of lines (default 40)")
    logs_p.add_argument("-f", action="store_true", help="Follow log output (like tail -f)")

    sub.add_parser("models",  help="List available models")

    test_p = sub.add_parser("test", help="Send 'tell me a joke' to a model")
    test_p.add_argument("model", nargs="?", default="claude-haiku-4-5",
                        help="Model slug (default: claude-haiku-4-5)")

    sub.add_parser("regen-models", help="Refresh ~/.codex/zen_models.json")

    args = p.parse_args()
    dispatch = {
        "start":        cmd_start,
        "stop":         cmd_stop,
        "restart":      cmd_restart,
        "status":       cmd_status,
        "logs":         cmd_logs,
        "models":       cmd_models,
        "test":         cmd_test,
        "regen-models": cmd_regen_models,
    }
    dispatch[args.cmd](args)


if __name__ == "__main__":
    main()
