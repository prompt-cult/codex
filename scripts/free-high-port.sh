#!/usr/bin/env bash
# Print a free high TCP port on 127.0.0.1 (macOS/Linux, no deps beyond python3).
set -euo pipefail
python3 - <<'PY'
import socket

s = socket.socket()
try:
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    # Keep it in the high ephemeral range; retry if the OS handed us a low one.
    while port < 20000:
        s.close()
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    print(port)
finally:
    s.close()
PY
