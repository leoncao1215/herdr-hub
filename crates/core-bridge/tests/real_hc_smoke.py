#!/usr/bin/env python3
"""Opt-in real hc smoke test using only uniquely named scratch sessions."""

import argparse
import asyncio
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import struct
import sys
import tempfile


async def execute(argv, environment, check=True):
    process = await asyncio.create_subprocess_exec(
        *map(str, argv), env=environment,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )
    try:
        stdout, stderr = await asyncio.wait_for(process.communicate(), 30)
    except BaseException:
        if process.returncode is None:
            process.kill()
        await process.wait()
        raise
    if check and process.returncode:
        raise RuntimeError(f"test command failed: {stderr.decode(errors='replace')[-1000:]}")
    return process.returncode, stdout


async def hc_json(hc, environment, *arguments):
    _, stdout = await execute([hc, "--json", *arguments], environment)
    return json.loads(stdout)


def bootstrap(session):
    return (
        "sh -c 'p=$(command -v herdr-core) || p=$HOME/.local/bin/herdr-core; "
        '[ -x "$p" ] || { echo "herdr-core is not installed here" >&2; exit 127; }; '
        f'HC_SESSION={session} exec "$p" connect\''
    )


async def roundtrip(bridge, hc, environment, processes):
    process = await asyncio.create_subprocess_exec(
        str(bridge), "local-bridge", "--hc", str(hc),
        "--session", environment["HC_SESSION"], "--", bootstrap(environment["HC_SESSION"]),
        env=environment, stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE, stderr=None,
    )
    processes.append(process)

    async def plain(request):
        process.stdin.write(json.dumps(request).encode() + b"\n")
        await asyncio.wait_for(process.stdin.drain(), 10)
        line = await asyncio.wait_for(process.stdout.readline(), 10)
        response = json.loads(line)
        if response.get("id") != request["id"] or "result" not in response:
            raise RuntimeError("unexpected plain RPC response")
        return response["result"]

    listed = await plain({"jsonrpc": "2.0", "id": 1, "method": "terminal.list", "params": {}})
    if not any(item.get("name") == "bridge-survivor" for item in listed["terminals"]):
        raise RuntimeError("test terminal missing from plain RPC")
    await plain({"jsonrpc": "2.0", "id": 2, "method": "hello", "params": {"framed": True}})
    request = json.dumps({
        "jsonrpc": "2.0", "id": 3, "method": "terminal.list", "params": {},
    }).encode()
    request += b" " * (255 - len(request))
    frame = struct.pack("<IBI", 0, 0, len(request)) + request
    for offset in range(0, len(frame), 17):
        process.stdin.write(frame[offset:offset + 17])
        await asyncio.wait_for(process.stdin.drain(), 10)
    header = await asyncio.wait_for(process.stdout.readexactly(9), 10)
    channel, kind, length = struct.unpack("<IBI", header)
    if channel != 0 or kind != 0 or length > 16 * 1024 * 1024:
        raise RuntimeError("invalid framed response header")
    reply = json.loads(await asyncio.wait_for(process.stdout.readexactly(length), 10))
    if reply.get("id") != 3 or "result" not in reply:
        raise RuntimeError("unexpected framed RPC response")
    process.stdin.close()
    code = await asyncio.wait_for(process.wait(), 10)
    if code != 0:
        raise RuntimeError("bridge did not close successfully after stdin EOF")


async def smoke(hc, bridge):
    root = Path(tempfile.mkdtemp(prefix="hubg2-"))
    session = "g2-" + secrets.token_hex(8)
    processes = []
    core_environments = []
    failure = None
    report = {}
    try:
        for directory in ("home", "run", "state", "config", "data"):
            (root / directory).mkdir(mode=0o700)
        environment = {
            key: value for key, value in os.environ.items()
            if key.upper() in {"PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP"}
        }
        environment.update({
            "HOME": str(root / "home"), "USERPROFILE": str(root / "home"),
            "APPDATA": str(root / "config"), "LOCALAPPDATA": str(root / "data"),
            "XDG_RUNTIME_DIR": str(root / "run"), "XDG_STATE_HOME": str(root / "state"),
            "XDG_CONFIG_HOME": str(root / "config"), "XDG_DATA_HOME": str(root / "data"),
            "HC_SESSION": session,
        })
        if sys.platform == "linux":
            environment["PATH"] = "/usr/local/bin:/usr/bin:/bin"
            environment["SHELL"] = "/bin/sh"
        _, version = await execute([hc, "--version"], environment)
        core_environments.append(environment)
        await hc_json(hc, environment, "server", "start")
        before = await hc_json(hc, environment, "server", "status")
        terminal = await hc_json(
            hc, environment, "run", "--bg", "--no-parent", "--name", "bridge-survivor",
            "--", sys.executable, "-c", "import time; print('ready', flush=True); time.sleep(120)",
        )
        address = terminal["address"]
        await roundtrip(bridge, hc, environment, processes)
        survivor = await hc_json(hc, environment, "get", address)
        if survivor["state"] != "live":
            raise RuntimeError("original terminal ended with the connector")
        await roundtrip(bridge, hc, environment, processes)

        via_verified = False
        if sys.platform == "linux":
            initiator = dict(environment, HC_SESSION="g2c-" + secrets.token_hex(8))
            core_environments.append(initiator)
            connector = shlex.join([
                str(bridge), "local-bridge", "--hc", str(hc), "--session", session, "--",
            ])
            add = ["machine", "add", "--temp", "--name", "bridge-smoke",
                   "--via", connector, "--session", session]
            added = await hc_json(hc, initiator, *add)
            if added["id"] != before["core"]["id"]:
                raise RuntimeError("connector reached the wrong hc session")
            _, binary = await execute([
                hc, "exec", "--on", "bridge-smoke", sys.executable,
                "-c", "import sys; sys.stdout.buffer.write(bytes(range(256)))",
            ], initiator)
            if binary != bytes(range(256)):
                raise RuntimeError("hc exec binary output changed through the connector")
            await hc_json(hc, initiator, "machine", "rm", "bridge-smoke")
            kept = await hc_json(hc, environment, "get", address)
            if kept["state"] != "live":
                raise RuntimeError("terminal ended when its machine connection was removed")
            added_again = await hc_json(hc, initiator, *add)
            if added_again["id"] != before["core"]["id"]:
                raise RuntimeError("reconnect replaced the remote core")
            kept = await hc_json(hc, initiator, "get", "bridge-smoke/" + address)
            if kept["state"] != "live":
                raise RuntimeError("terminal did not survive connector reconnection")
            await hc_json(hc, initiator, "machine", "rm", "bridge-smoke")
            via_verified = True

        after = await hc_json(hc, environment, "server", "status")
        if (after["core"]["id"], after["pid"]) != (before["core"]["id"], before["pid"]):
            raise RuntimeError("connector disconnect restarted the remote core")
        report = {
            "platform": sys.platform, "hc_version": version.decode().strip(),
            "plain_rpc": True, "framed_rpc": True, "original_terminal_survived": True,
            "core_not_restarted": True, "native_linux_via_and_binary_exec": via_verified,
        }
    except BaseException as error:
        failure = error

    cleanup_errors = []
    for process in processes:
        if process.returncode is None:
            process.kill()
        try:
            await asyncio.wait_for(process.wait(), 10)
        except Exception as error:
            cleanup_errors.append(error)
    for environment in reversed(core_environments):
        try:
            if not environment["HC_SESSION"].startswith(("g2-", "g2c-")):
                raise RuntimeError("refusing to stop a non-test core")
            if environment["XDG_STATE_HOME"] != str(root / "state"):
                raise RuntimeError("refusing to stop a core outside the scratch state")
            await hc_json(hc, environment, "server", "stop")
        except Exception as error:
            cleanup_errors.append(error)
    if not cleanup_errors:
        shutil.rmtree(root)
    if failure is not None:
        if cleanup_errors:
            raise failure from RuntimeError("scratch cleanup also failed; state retained")
        raise failure
    if cleanup_errors:
        raise RuntimeError("scratch cleanup failed; state retained") from cleanup_errors[0]
    report["isolated_cores_stopped_and_scratch_removed"] = True
    print(json.dumps(report, indent=2))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--hc", type=Path, required=True)
    parser.add_argument("--bridge", type=Path, required=True)
    arguments = parser.parse_args()
    if sys.platform == "linux" and os.getuid() == 0:
        raise SystemExit("Run as the non-root test user")
    hc = arguments.hc.resolve(strict=True)
    bridge = arguments.bridge.resolve(strict=True)
    asyncio.run(smoke(hc, bridge))


if __name__ == "__main__":
    main()
