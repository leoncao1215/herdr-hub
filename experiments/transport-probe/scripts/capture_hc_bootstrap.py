#!/usr/bin/env python3
"""Capture hc connector arguments without evaluating them or contacting a peer."""

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile


def main():
    if sys.platform != "linux" or os.getuid() == 0:
        raise RuntimeError("Run as the dedicated non-root Linux test user")
    hc = Path(sys.argv[1]).resolve(strict=True)
    if not hc.is_file():
        raise RuntimeError("Expected an explicit Linux hc executable")

    parent = Path(tempfile.gettempdir())
    root = Path(tempfile.mkdtemp(prefix="hubcap-", dir=parent))
    core_attempted = False
    try:
        for name in ("home", "run", "state", "config"):
            (root / name).mkdir(mode=0o700)
        environment = {
            "PATH": "/usr/local/bin:/usr/bin:/bin",
            "HOME": str(root / "home"),
            "SHELL": "/bin/sh",
            "LANG": "C.UTF-8",
            "HC_SESSION": "probe",
            "XDG_RUNTIME_DIR": str(root / "run"),
            "XDG_STATE_HOME": str(root / "state"),
            "XDG_CONFIG_HOME": str(root / "config"),
        }
        connector = root / "connector.py"
        connector.write_text(
            "import json, sys\n"
            "with open(sys.argv[1], 'a', encoding='utf-8') as output:\n"
            "    output.write(json.dumps(sys.argv[2:]) + '\\n')\n"
            "print('Connection timed out (capture-only connector)', file=sys.stderr)\n"
            "sys.exit(255)\n",
            encoding="utf-8",
        )
        version = subprocess.run(
            [str(hc), "--version"], env=environment, check=True,
            capture_output=True, text=True, timeout=15,
        ).stdout.strip()
        captures = []
        for index, session in enumerate((None, "hub-probe", "hub.probe-1")):
            output = root / f"capture-{index}.jsonl"
            command = [
                str(hc), "--json", "machine", "add",
                "--temp", "--name", f"capture-{index}",
                "--via", shlex.join(["/usr/bin/python3", str(connector), str(output)]),
            ]
            if session is not None:
                command.extend(["--session", session])
            core_attempted = True
            result = subprocess.run(
                command, env=environment, capture_output=True, text=True, timeout=60,
            )
            if result.returncode == 0 or not output.exists():
                raise RuntimeError(f"Capture failed: {result.stderr[-500:]}")
            invocations = [json.loads(line) for line in output.read_text().splitlines()]
            if len(invocations) != 1 or len(invocations[0]) != 1:
                raise RuntimeError("Expected exactly one connection-only bootstrap argument")
            error = json.loads(result.stderr)
            if error["error"]["kind"] != "unreachable":
                raise RuntimeError(f"Unexpected capture result: {error['error']['kind']}")
            captures.append({
                "remote_session": session,
                "connector_arguments": invocations[0],
                "expected_error": error["error"]["kind"],
            })
    finally:
        if core_attempted:
            stopped = subprocess.run(
                [str(hc), "--json", "server", "stop"], env=environment,
                capture_output=True, text=True, timeout=30,
            )
            if stopped.returncode != 0:
                raise RuntimeError(
                    "Isolated core cleanup failed; scratch state retained for diagnosis"
                )
        if root.parent != parent or not root.name.startswith("hubcap-"):
            raise RuntimeError("Refusing to remove unexpected scratch location")
        shutil.rmtree(root)
    print(json.dumps({
        "hc_version": version,
        "platform": "Linux under WSL2",
        "connector_behavior": "record argv, never execute bootstrap, exit 255",
        "cases": captures,
        "cleanup": "isolated Linux core stopped; scratch state removed",
    }, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
