"""User category rules: site-beats-extension classification and persistence."""

import json
import os
import socket
import struct
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def exchange(writer, request, reader=None):
    reader = reader or writer
    data = json.dumps(request).encode()
    writer.write(struct.pack("=I", len(data)) + data)
    writer.flush()
    size = struct.unpack("=I", reader.read(4))[0]
    return json.loads(reader.read(size))


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-categories-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")}
        (base / "home").mkdir()
        daemon = None
        try:
            daemon = subprocess.Popen(
                [str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            token = (base / "state/ipc-token").read_text()

            def rpc(kind, **payload):
                with socket.socket(socket.AF_UNIX) as stream:
                    stream.settimeout(10)
                    stream.connect(str(base / "state/daemon.sock"))
                    # Unit commands such as `category_list` carry no payload.
                    command = {"type": kind, **({"payload": payload} if payload else {})}
                    return exchange(stream.makefile("rwb"), {"version": 1, "token": token, "command": command})

            assert rpc("category_save", name="Ebooks", extensions=["epub", "mobi"],
                       destination=str(base / "library"), sites=[])["ok"]
            assert rpc("category_save", name="PreviewHost", extensions=[],
                       destination=str(base / "preview"), sites=["*.preview.test"])["ok"]
            edited = rpc("category_save", name="Documents", extensions=["pdf"], destination="")
            assert edited["ok"], edited
            assert not rpc("category_save", name="", extensions=["pdf"], destination="/x", sites=[])["ok"]
            assert not rpc("category_save", name="Bad", extensions=[], destination="rel/dir", sites=[])["ok"]

            rules = {rule["name"]: rule for rule in rpc("category_list")["categories"]}
            for builtin in ["Compressed", "Documents", "Music", "Programs", "Video", "Other"]:
                assert builtin in rules, builtin
            assert rules["Ebooks"]["destination"] == str(base / "library"), rules
            assert rules["Compressed"]["extensions"], "unedited built-ins keep their list"
            assert rules["PreviewHost"]["sites"] == ["*.preview.test"]
            assert rules["Documents"]["extensions"] == ["pdf"], "edit replaces the built-in table"
            def submit(request_id, filename, host):
                reply = rpc("submit", request_id=request_id, url=f"http://{host}/x/{filename}",
                            output=str(base / "dest" / filename), headers=[], priority=1, connections=4)
                assert reply["ok"], reply
                return reply["task_id"]

            def categories_by_id(task_ids):
                rows = {row["id"]: row for row in rpc("list")["tasks"]}
                return {request_id: rows[task_id]["category"] for request_id, task_id in task_ids.items()}

            task_ids = {}
            task_ids["cat-site-first"] = submit("cat-site-first", "papers.epub", "www.preview.test")
            task_ids["cat-extension"] = submit("cat-extension", "book.epub", "plain.test")
            task_ids["cat-edited"] = submit("cat-edited", "notes.txt", "plain.test")
            # A user extension rule can re-assign built-in behaviour.
            assert rpc("category_save", name="Music", extensions=["mp3", "txt"], destination="", sites=[])["ok"]
            task_ids["cat-reassigned"] = submit("cat-reassigned", "other.txt", "plain.test")
            observed = categories_by_id(task_ids)
            # A host match beats an extension match on the same submission.
            assert observed["cat-site-first"] == "PreviewHost", observed
            assert observed["cat-extension"] == "Ebooks", observed
            assert observed["cat-edited"] == "Documents", observed
            assert observed["cat-reassigned"] == "Music", observed

            # Built-in names can be edited but never removed.
            assert rpc("category_save", name="Video", extensions=["mp4", "webm"], destination="", sites=[])["ok"]
            assert not rpc("category_remove", name="Video")["ok"]
            assert not rpc("category_remove", name="Nobody")["ok"]
            assert rpc("category_remove", name="PreviewHost")["ok"]
            task_ids["cat-after-removal"] = submit("cat-after-removal", "papers2.epub", "www.preview.test")
            fresh = categories_by_id(task_ids)
            assert fresh["cat-after-removal"] == "Ebooks", fresh

            daemon.terminate(); daemon.wait(timeout=5)
            daemon = subprocess.Popen(
                [str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            token = (base / "state/ipc-token").read_text()
            names = [rule["name"] for rule in rpc("category_list")["categories"]]
            assert "Ebooks" in names and "PreviewHost" not in names, names
            after = {}
            after["cat-after-restart"] = rpc("submit", request_id="cat-after-restart", url="http://plain.test/x/photo.epub",
                                             output=str(base / "dest/photo.epub"), headers=[], priority=1, connections=4)["task_id"]
            rows = {row["id"]: row for row in rpc("list")["tasks"]}
            assert rows[after["cat-after-restart"]]["category"] == "Ebooks", rows
            print("user categories, site rule precedence, built-in edit protection, removal and restart persistence passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)


if __name__ == "__main__":
    main()
