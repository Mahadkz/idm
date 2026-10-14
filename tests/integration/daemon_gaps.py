"""ListPage paging over 20k rows, completed-file move, orphan partial cleanup
and the native host health message, against real binaries and disposable
local servers/data directories."""
import json
import os
import socket
import sqlite3
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import contracts_behavior as cb  # noqa: E402

MIB = cb.MIB


def rpc(state, kind, **payload):
    request = {"version": 1, "token": (state / "ipc-token").read_text(),
               "command": {"type": kind, "payload": payload}}
    data = json.dumps(request).encode()
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(30)
        stream.connect(str(state / "daemon.sock"))
        stream.sendall(struct.pack("=I", len(data)) + data)
        with stream.makefile("rb") as reader:
            size = struct.unpack("=I", reader.read(4))[0]
            return json.loads(reader.read(size))


def finish(daemon, task_id):
    daemon.wait_status(task_id, ("complete",), seconds=15)


def test_paging(root):
    state = root / "state"
    daemon = cb.Daemon(root / "home", state)
    daemon.wait_listening()
    daemon.terminate()
    db = sqlite3.connect(state / "tasks.sqlite3")
    rows = [(f"r{i}", f"http://127.0.0.1:9/f{i}.bin", f"/nonexistent/f{i}.bin", "[]", 1, 1,
             "complete" if i % 4 else "failed", "Video" if i % 3 == 0 else "Other", 0, 1)
            for i in range(1, 20001)]
    db.executemany("INSERT INTO tasks(request_id,url,output,headers_json,priority,connections,status,category,queue_id,history_queue_id) VALUES (?,?,?,?,?,?,?,?,?,?)", rows)
    db.commit()
    db.close()
    daemon = cb.Daemon(root / "home", state)
    try:
        daemon.wait_listening()
        legacy = daemon.cli("list")
        assert len(legacy["tasks"]) == 1000 and legacy["total"] == 0
        started = time.perf_counter()
        page = rpc(state, "list_page", offset=0, limit=2000)
        ms = (time.perf_counter() - started) * 1000
        assert page["ok"] and len(page["tasks"]) == 2000 and page["total"] == 20000, page.get("error")
        assert page["tasks"][0]["id"] == 20000
        started = time.perf_counter()
        deep = rpc(state, "list_page", offset=18000, limit=2000, sort="url")
        deep_ms = (time.perf_counter() - started) * 1000
        ids = [t["id"] for t in deep["tasks"]]
        assert len(ids) == 2000 and deep["total"] == 20000
        # stable: consecutive pages never overlap and cover everything once
        seen = set()
        for offset in range(0, 20000, 2000):
            for task in rpc(state, "list_page", offset=offset, limit=2000, sort="category")["tasks"]:
                seen.add(task["id"])
        assert len(seen) == 20000
        filtered = rpc(state, "list_page", offset=0, limit=50,
                       filter={"status": "failed", "category": "Video", "search": "f1"})
        assert filtered["ok"] and 0 < filtered["total"] < 5000
        assert all(t["status"] == "failed" and t["category"] == "Video" and "f1" in t["url"] for t in filtered["tasks"])
        assert not rpc(state, "list_page", offset=0, limit=2001)["ok"]
        assert not rpc(state, "list_page", offset=0, limit=0)["ok"]
        assert not rpc(state, "list_page", offset=0, limit=5, sort="nope; drop")["ok"]
        cli = daemon.cli("list", "--offset", 5, "--limit", 3)
        assert [t["id"] for t in cli["tasks"]] == [19995, 19994, 19993] and cli["total"] == 20000
        print(f"ListPage 20k rows: limit 2000 first page {ms:.1f} ms, offset 18000 sorted by url {deep_ms:.1f} ms")
    finally:
        daemon.terminate()


def test_move(root):
    server = cb.Fixture()
    server.set("/a.bin", size=64 * 1024)
    server.set("/big.bin", size=16 * MIB)
    try:
        daemon = cb.Daemon(root / "home", root / "state")
        try:
            daemon.wait_listening()
            work = root / "dl"
            work.mkdir()
            tid = daemon.submit(server.url("/a.bin"), work / "a.bin", wait=False)
            finish(daemon, tid)
            data = (work / "a.bin").read_bytes()
            dest = work / "sub"
            dest.mkdir()
            reply = daemon.cli("move-file", tid, dest / "moved.bin")
            assert reply["ok"] and not (work / "a.bin").exists()
            assert (dest / "moved.bin").read_bytes() == data
            assert daemon.info(tid)["output"] == str(dest / "moved.bin")
            # never overwrite
            (work / "taken.bin").write_bytes(b"keep")
            daemon.cli("move-file", tid, work / "taken.bin", success=False)
            assert (work / "taken.bin").read_bytes() == b"keep" and (dest / "moved.bin").exists()
            daemon.cli("move-file", tid, work / "nodir" / "x.bin", success=False)
            daemon.cli("move-file", tid, "relative.bin", success=False)
            # cross-volume copy path
            shm = Path("/dev/shm")
            if shm.is_dir() and os.stat(shm).st_dev != os.stat(work).st_dev:
                target = shm / f"xdm-move-{os.getpid()}.bin"
                try:
                    assert daemon.cli("move-file", tid, target)["ok"]
                    assert target.read_bytes() == data and not (dest / "moved.bin").exists()
                    assert not list(shm.glob(f"xdm-move-{os.getpid()}*.tmp"))
                finally:
                    target.unlink(missing_ok=True)
            # running tasks refused
            run = daemon.cli("submit", server.url("/big.bin"), str(work / "big.bin"))["task_id"]
            daemon.cli("rate", run, 64)
            daemon.wait_status(run, ("running",), seconds=8)
            daemon.cli("move-file", run, work / "big2.bin", success=False)
            daemon.cli("pause", run)
            daemon.wait_status(run, ("paused",), seconds=8)
            daemon.cli("move-file", run, work / "big2.bin", success=False)  # not complete
            print("move-file: rename, no-overwrite, cross-volume, running/incomplete refusal passed")
        finally:
            daemon.terminate()
    finally:
        server.close()


def test_partials(root):
    server = cb.Fixture()
    server.set("/big.bin", size=16 * MIB)
    try:
        daemon = cb.Daemon(root / "home", root / "state")
        try:
            daemon.wait_listening()
            work = root / "dl"
            work.mkdir()
            tid = daemon.cli("submit", server.url("/big.bin"), str(work / "big.bin"))["task_id"]
            daemon.cli("rate", tid, 64)
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline and not (work / "big.bin.part.meta").exists():
                time.sleep(0.05)
            daemon.cli("pause", tid)
            daemon.wait_status(tid, ("paused",), seconds=8)
            # owned partial is not listed
            (work / "ghost.bin.part").write_bytes(b"x" * 100)
            (work / "ghost.bin.part.meta").write_bytes(b"{}")
            (work / "half.bin.part").write_bytes(b"y" * 7)
            outside = root / "outside.txt"
            outside.write_text("precious")
            os.symlink(outside, work / "link.bin.part")
            scan = daemon.cli("partials", work)
            found = {Path(e["output"]).name: e for e in scan["partials"]}
            assert set(found) == {"ghost.bin", "half.bin"}, found
            assert found["ghost.bin"]["part_size"] == 100 and found["ghost.bin"]["meta_size"] == 2
            assert found["half.bin"]["meta"] is None
            # delete only scanned paths
            daemon.cli("partials-delete", work, work / "big.bin.part", success=False)
            daemon.cli("partials-delete", work, outside, success=False)
            daemon.cli("partials-delete", work, work / "link.bin.part", success=False)
            assert (work / "big.bin.part").exists() and outside.exists()
            daemon.cli("partials-delete", work, found["ghost.bin"]["part"], found["ghost.bin"]["meta"])
            assert not (work / "ghost.bin.part").exists() and not (work / "ghost.bin.part.meta").exists()
            assert (work / "half.bin.part").exists()
            # remove keeps parts by default, deletes own parts on request
            daemon.cli("remove", tid, "--delete-partial")
            leftovers = sorted(p.name for p in work.iterdir())
            assert leftovers == ["half.bin.part", "link.bin.part"], leftovers
            assert outside.read_text() == "precious"
            print("partials: scan, symlink skip, scan-gated delete, remove --delete-partial passed")
        finally:
            daemon.terminate()
    finally:
        server.close()


def test_health(root):
    native = cb.ROOT / "target/debug/xdm-native-host"
    home = root / "home"
    manifests = home / ".mozilla/native-messaging-hosts"
    manifests.mkdir(parents=True)
    (manifests / "org.xdm.browser.json").write_text(json.dumps(
        {"name": "org.xdm.browser", "path": str(native), "type": "stdio", "allowed_extensions": ["x@y"]}))
    env = {**os.environ, "XDM_DATA_DIR": str(root / "state"), "HOME": str(home)}

    def ask():
        frame = json.dumps({"version": 1, "type": "health"}).encode()
        result = subprocess.run([str(native)], input=struct.pack("=I", len(frame)) + frame,
                                env=env, capture_output=True, timeout=15)
        (size,) = struct.unpack("=I", result.stdout[:4])
        return json.loads(result.stdout[4:4 + size])

    down = ask()
    assert down["type"] == "health.result" and down["daemon"]["reachable"] is False
    assert "daemon" in down["failing"] and down["manifest"]["registered"] is True
    entry = down["manifest"]["entries"][0]
    assert entry["browser"] == "firefox" and entry["hostPathIsThisBinary"] is True
    daemon = cb.Daemon(home, root / "state")
    try:
        daemon.wait_listening()
        up = ask()
        assert up["daemon"]["reachable"] is True and up["daemon"]["version"], up
        assert up["failing"] == [] and up["ok"] is True and up["nativeHost"]["version"]
    finally:
        daemon.terminate()
    (manifests / "org.xdm.browser.json").unlink()
    assert ask()["manifest"]["registered"] is False and "manifest" in ask()["failing"]
    print("native host health: daemon down/up and manifest present/missing passed")


def main():
    for test in (test_paging, test_move, test_partials, test_health):
        with tempfile.TemporaryDirectory(prefix="xdm-gaps-") as temp:
            test(Path(temp))
    print("daemon_gaps passed")


if __name__ == "__main__":
    main()
