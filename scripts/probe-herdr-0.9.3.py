#!/usr/bin/env python3
"""Isolated upstream compatibility probe, not the Relay implementation.

Uses Herdr 0.9.3 test-protocol fixtures from commit
7b116c05bfda646af39d2524c54e70c751f57ee8. Only operates on processes and
sockets created under a fresh temporary directory; never uses a default socket.
Run: python3 scripts/probe-herdr-0.9.3.py /absolute/path/to/herdr-0.9.3
"""

import argparse
import hashlib
import json
import os
import platform
import socket
import struct
import subprocess
import tempfile
import threading
import time
from pathlib import Path


def vi(value):
    if value < 251:
        return bytes([value])
    if value <= 65535:
        return b"\xfb" + struct.pack("<H", value)
    return b"\xfc" + struct.pack("<I", value)


def string(value):
    data = value.encode()
    return vi(len(data)) + data


def decode_vi(data, pos=0):
    tag = data[pos]
    if tag < 251:
        return tag, pos + 1
    size, fmt = {251: (2, "<H"), 252: (4, "<I")}[tag]
    return struct.unpack(fmt, data[pos + 1:pos + 1 + size])[0], pos + 1 + size


def decode_string(data, pos=0):
    length, pos = decode_vi(data, pos)
    return data[pos:pos + length].decode(), pos + length


def send(sock, data):
    sock.sendall(struct.pack("<I", len(data)) + data)


def wait_until(predicate, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("timed out waiting for expected condition")


class Server:
    def __init__(self, binary, base):
        self.base = base
        for name in ("config/herdr", "runtime", "state", "cache"):
            (base / name).mkdir(parents=True, exist_ok=True)
        (base / "config/herdr/config.toml").write_text("onboarding = false\n")
        self.api = base / "runtime/herdr.sock"
        self.client = base / "runtime/herdr-client.sock"
        env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
        env.update(XDG_CONFIG_HOME=str(base / "config"), XDG_STATE_HOME=str(base / "state"),
                   XDG_CACHE_HOME=str(base / "cache"), XDG_RUNTIME_DIR=str(base / "runtime"),
                   HERDR_SOCKET_PATH=str(self.api), SHELL="/bin/sh", HERDR_DISABLE_SOUND="1")
        self.log = (base / "process.log").open("wb")
        self.proc = subprocess.Popen([str(binary), "server"], cwd=base, env=env,
                                     stdin=subprocess.DEVNULL, stdout=self.log,
                                     stderr=self.log, start_new_session=True)

    def request(self, method, **params):
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(4)
            sock.connect(str(self.api))
            sock.sendall(json.dumps(dict(id="probe", method=method, params=params)).encode() + b"\n")
            with sock.makefile("rb") as file:
                response = json.loads(file.readline())
        assert "error" not in response, response
        return response["result"]

    def create_pane(self):
        return self.request("workspace.create", label="relay-probe")["root_pane"]["pane_id"]

    def read_pane(self, pane):
        return self.request("pane.read", pane_id=pane, source="recent", lines=200)["read"]["text"]

    def api_input(self, pane, text):
        self.request("pane.send_input", pane_id=pane, text=text, keys=["Enter"])

    def size(self, pane):
        marker = "SIZE_" + str(time.monotonic_ns())
        self.api_input(pane, "echo " + marker + "; stty size")
        def inspect():
            lines = self.read_pane(pane).splitlines()
            for index, line in enumerate(lines):
                if line.strip() == marker and index + 1 < len(lines):
                    words = lines[index + 1].strip().split()
                    if len(words) == 2 and all(w.isdigit() for w in words):
                        return tuple(map(int, words))
            return None
        return wait_until(inspect)

    def close(self):
        if self.proc.poll() is None:
            try:
                self.request("server.stop")
                self.proc.wait(timeout=5)
            except (OSError, ValueError, AssertionError, subprocess.TimeoutExpired):
                self.proc.terminate()
                try:
                    self.proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                    self.proc.wait()
        self.log.close()


class Client:
    def __init__(self, path, cols, rows):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(0.2)
        self.sock.connect(str(path))
        self.messages = []
        self.error = None
        self.closed = False
        hello = dict(generation=1, cell_width_px=8, cell_height_px=16,
                     surface_size=dict(cols=cols, rows=rows), pixel_mouse=False,
                     direct_graphics=False, endpoint_keybindings=False, mouse_capture=False,
                     snapshot_codecs=["shell.snapshot.v1"], surface_codecs=["shell.surface.v1"],
                     input_codecs=["shell.input.semantic.v1"], blob_codecs=["shell.blob.v1"])
        send(self.sock, vi(20) + string("endpoint.hello.v1") + string(json.dumps(hello)))
        self.thread = threading.Thread(target=self.read, daemon=True)
        self.thread.start()

    def read(self):
        buf = bytearray()
        try:
            while not self.closed:
                try:
                    part = self.sock.recv(65536)
                except socket.timeout:
                    continue
                if not part:
                    return
                buf.extend(part)
                while len(buf) >= 4:
                    length = struct.unpack("<I", buf[:4])[0]
                    assert 0 < length <= 32 * 1024 * 1024, length
                    if len(buf) < 4 + length:
                        break
                    data = bytes(buf[4:4 + length])
                    del buf[:4 + length]
                    variant, pos = decode_vi(data)
                    kind, value = None, None
                    if variant == 20:
                        kind, pos = decode_string(data, pos)
                        text, pos = decode_string(data, pos)
                        value = json.loads(text)
                    self.messages.append((variant, kind, value))
        except Exception as exc:
            if not self.closed:
                self.error = repr(exc)

    def ready(self):
        welcomes = [v for _, k, v in self.messages if k == "endpoint.welcome.v1"]
        if welcomes:
            assert welcomes[0].get("error") is None, welcomes[0]
        return (bool(welcomes) and any(k == "shell.snapshot.v1" for _, k, _ in self.messages)
                and any(v == 13 for v, _, _ in self.messages))

    def surface_count(self):
        return sum(v in (13, 19) for v, _, _ in self.messages)

    def input(self, pane, text):
        text_event = vi(1) + string(text)
        enter_event = vi(0) + vi(1) + b"\x00" + vi(0) + vi(1) + b"\x00" * 5
        send(self.sock, vi(13) + string(pane) + vi(2) + text_event + enter_event)

    def close(self):
        self.closed = True
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.sock.close()
        self.thread.join(timeout=1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    assert version == "herdr 0.9.3", version
    results = dict(version=version, platform=platform.platform(),
                   binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), checks={})
    servers, clients = [], []
    with tempfile.TemporaryDirectory(prefix="hrprobe-", dir="/tmp") as temporary:
        base = Path(temporary)
        try:
            a = Server(binary, base / "a")
            servers.append(a)
            wait_until(lambda: a.api.exists() and a.client.exists())
            pane = a.create_pane()
            large = Client(a.client, 120, 40)
            clients.append(large)
            wait_until(large.ready)
            small = Client(a.client, 80, 24)
            clients.append(small)
            wait_until(small.ready)
            results["checks"]["two_native_clients_bootstrap"] = True
            initial = a.size(pane)
            assert initial[0] > 24 and initial[1] > 80, initial
            results["checks"]["passive_attach_preserves_geometry"] = list(initial)
            send(small.sock, vi(18) + b"\x01")
            reduced = wait_until(lambda: (s if (s := a.size(pane))[0] <= 24 and s[1] <= 80 else None))
            results["checks"]["focus_moves_geometry_control"] = list(reduced)
            for client, marker in ((large, "NATIVE_A_123"), (small, "NATIVE_B_456")):
                before = (large.surface_count(), small.surface_count())
                client.input(pane, "printf '" + marker + "\\n'")
                wait_until(lambda: any(line.strip() == marker for line in a.read_pane(pane).splitlines()))
                wait_until(lambda: large.surface_count() > before[0] and small.surface_count() > before[1])
            results["checks"]["both_clients_input_and_receive_updates"] = True
            small.close()
            restored = wait_until(lambda: (s if (s := a.size(pane))[0] > 24 and s[1] > 80 else None))
            results["checks"]["peer_disconnect_restores_survivor_geometry"] = list(restored)
            before = large.surface_count()
            large.input(pane, "printf 'SURVIVOR_789\\n'")
            wait_until(lambda: any(line.strip() == "SURVIVOR_789" for line in a.read_pane(pane).splitlines()))
            wait_until(lambda: large.surface_count() > before)
            assert a.proc.poll() is None
            results["checks"]["disconnect_keeps_server_and_survivor"] = True
            b = Server(binary, base / "b")
            servers.append(b)
            wait_until(lambda: b.api.exists() and b.client.exists())
            second_pane = b.create_pane()
            second = Client(b.client, 100, 30)
            clients.append(second)
            wait_until(second.ready)
            second.input(second_pane, "printf 'SECOND_INSTANCE_987\\n'")
            wait_until(lambda: any(line.strip() == "SECOND_INSTANCE_987" for line in b.read_pane(second_pane).splitlines()))
            assert "SECOND_INSTANCE_987" not in a.read_pane(pane)
            results["checks"]["separate_instance_socket_isolation"] = True
            assert not any(c.error for c in clients), [c.error for c in clients]
        finally:
            for client in clients:
                client.close()
            for server in reversed(servers):
                server.close()
            results["created_processes_exited"] = all(s.proc.poll() is not None for s in servers)
    print(json.dumps(results, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
