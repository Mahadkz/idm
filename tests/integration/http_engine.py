"""HTTP engine request adoption, one-shot links, method replay, resume and publication; run after cargo build -p xdm-cli."""

import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "target/debug/xdm-cli"
FIXTURE = Path(__file__).resolve().parent / "http_fixture.py"
PROXY_FIXTURE = Path(__file__).resolve().parent / "proxy_fixture.py"
SIZE = 4 * 1024 * 1024
BLOCK = bytes(range(256)) * 256
EXPECTED = hashlib.sha256(BLOCK * (SIZE // len(BLOCK))).hexdigest()


def run(*args, success=True, env=None):
    result = subprocess.run([str(CLI), *map(str, args)], text=True, capture_output=True, timeout=90,
                            env={**os.environ, **(env or {})})
    assert (result.returncode == 0) == success, (args, result.returncode, result.stderr)
    return result


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def interrupted(url, output, *extra, after=0.4):
    worker = subprocess.Popen([str(CLI), url, str(output), *map(str, extra)],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    time.sleep(after)
    worker.send_signal(signal.SIGINT)
    stdout, stderr = worker.communicate(timeout=20)
    assert worker.returncode != 0 and "paused" in stderr, (worker.returncode, stdout, stderr)
    meta = Path(f"{output}.part.meta")
    assert meta.exists(), "pause must leave a resumable checkpoint"
    done = sum(s["cursor"] - s["start"] for s in json.loads(meta.read_text())["segments"])
    assert 0 < done < SIZE, done


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-engine-test-") as temp:
        directory = Path(temp)
        log = directory / "requests.jsonl"
        server = subprocess.Popen([sys.executable, str(FIXTURE), str(log)],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            port = json.loads(server.stdout.readline())["port"]
            base = f"http://127.0.0.1:{port}"

            def requests(path):
                return [r for r in map(json.loads, log.read_text().splitlines()) if r["path"] == path]

            # A ranged download adopts its probe response: no HEAD, and the first request carries data.
            out = directory / "file.bin"
            run(f"{base}/file.bin", out, "--connections", "8")
            assert digest(out) == EXPECTED
            seen = requests("/file.bin")
            assert all(r["method"] == "GET" for r in seen), seen
            assert seen[0]["range"] == "bytes=0-", seen[0]

            # One connection: exactly one request for the whole file.
            out = directory / "single.bin"
            run(f"{base}/single.bin", out, "--connections", "1")
            assert digest(out) == EXPECTED and len(requests("/single.bin")) == 1

            # A one-shot ranged link refuses extra connections; the first stream absorbs their ranges.
            out = directory / "oneshot.bin"
            run(f"{base}/oneshot.bin", out, "--connections", "8")
            assert digest(out) == EXPECTED
            assert sum(r["attempt"] == 1 for r in requests("/oneshot.bin")) == 1

            # Servers without ranges are streamed from the probe response itself.
            for name in ["norange.bin", "oneshot-norange.bin"]:
                out = directory / name
                run(f"{base}/{name}", out, "--connections", "8")
                assert digest(out) == EXPECTED and len(requests(f"/{name}")) == 1, name

            # A full response that disconnects halfway is retried from byte zero.
            out = directory / "flaky.bin"
            run(f"{base}/flaky-norange.bin", out)
            assert digest(out) == EXPECTED and len(requests("/flaky-norange.bin")) == 2

            out = directory / "empty.bin"
            run(f"{base}/empty.bin", out)
            assert out.read_bytes() == b""

            # POST bodies are replayed with every range request and survive 307, never 302.
            body = directory / "body.txt"
            body.write_bytes(b"token=abc")
            out = directory / "post.bin"
            run(f"{base}/post.bin", out, "--data-file", body, "--connections", "4")
            assert digest(out) == EXPECTED
            assert all(r["method"] == "POST" and r["body"] == "token=abc" for r in requests("/post.bin"))
            out = directory / "post-get.bin"
            run(f"{base}/post.bin", out, success=False)
            assert not out.exists()
            out = directory / "post-307.bin"
            run(f"{base}/post-307", out, "--data-file", body)
            assert digest(out) == EXPECTED
            out = directory / "post-302.bin"
            failed = run(f"{base}/post-302", out, "--data-file", body, success=False)
            assert "method" in failed.stderr and not out.exists(), failed.stderr

            # Cookies reach the original origin but are dropped on a cross-host redirect.
            target = f"http://localhost:{port}/redirected.bin"
            out = directory / "redirected.bin"
            run(f"{base}/cookie-redirect?to={target}", out, "--header", "Cookie: session=secret")
            assert digest(out) == EXPECTED
            assert requests("/cookie-redirect")[0]["cookie"] == "session=secret"
            assert all(r["cookie"] is None for r in requests("/redirected.bin"))

            # Pause and resume with one connection, then change connection counts across resumes.
            for first, second, name in [(1, 1, "resume-1-1"), (1, 4, "resume-1-4"), (4, 1, "resume-4-1")]:
                out = directory / f"{name}.bin"
                interrupted(f"{base}/{name}.bin", out, "--connections", first, after=0.4 / first)
                run(f"{base}/{name}.bin", out, "--connections", second)
                assert digest(out) == EXPECTED, name
                assert not Path(f"{out}.part.meta").exists()
                ranges = [r["range"] for r in requests(f"/{name}.bin")]
                assert ranges[1] != "bytes=0-", (name, ranges)

            # A file created at the destination during a transfer is never overwritten.
            out = directory / "race.bin"
            worker = subprocess.Popen([str(CLI), f"{base}/race.bin", str(out), "--connections", "1"],
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            time.sleep(0.3)
            out.write_bytes(b"keep")
            _, stderr = worker.communicate(timeout=30)
            assert worker.returncode != 0 and "appeared" in stderr, stderr
            assert out.read_bytes() == b"keep"
            assert digest(Path(f"{out}.part")) == EXPECTED

            # N31: a range connection that stops delivering is abandoned well before the
            # server's 30 s hold ends, and reopened from the stalled worker's cursor.
            out = directory / "stall.bin"
            started = time.monotonic()
            run(f"{base}/stall.bin", out, "--connections", "4")
            elapsed = time.monotonic() - started
            assert digest(out) == EXPECTED
            assert elapsed < 25, f"stall recovery took {elapsed:.1f}s"
            seen = requests("/stall.bin")
            assert len(seen) >= 5, seen
            closed = [tuple(int(v) for v in r["range"][len("bytes="):].split("-"))
                      for r in seen if r["range"] and not r["range"].endswith("-")]
            assert any(a > start and a <= end for start, end in closed for a, _ in closed), closed

            # N05: a server that stops honoring ranges fails with a typed error, stops
            # every worker and leaves a resumable checkpoint instead of mixed data.
            out = directory / "flip.bin"
            failed = run(f"{base}/flip.bin", out, "--connections", "4", success=False)
            assert "stopped honoring byte ranges" in failed.stderr, failed.stderr
            assert "--restart" in failed.stderr, failed.stderr
            assert not out.exists() and Path(f"{out}.part").exists()
            meta = json.loads(Path(f"{out}.part.meta").read_text())
            done = sum(s["cursor"] - s["start"] for s in meta["segments"])
            assert 0 < done < SIZE, done
            assert 3 <= len(requests("/flip.bin")) <= 4

            # A throttled transfer is not a stall: the limiter waits between reads
            # while the socket keeps delivering, so the cap holds for the whole file.
            out = directory / "throttle.bin"
            started = time.monotonic()
            run(f"{base}/throttle.bin", out, "--rate-kib", "256")
            elapsed = time.monotonic() - started
            assert digest(out) == EXPECTED and elapsed >= 14, elapsed

            # Proxy routing: environment by default, explicit direct, HTTP proxy, SOCKS5 with remote DNS, bypass.
            proxy_log = directory / "proxy.jsonl"
            proxies = subprocess.Popen([sys.executable, str(PROXY_FIXTURE), str(proxy_log)],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                ports = json.loads(proxies.stdout.readline())
                dead = {"http_proxy": "http://127.0.0.1:9", "no_proxy": "", "NO_PROXY": ""}
                run(f"{base}/env.bin", directory / "env.bin", env=dead, success=False)
                run(f"{base}/direct.bin", directory / "direct.bin", "--no-proxy", env=dead)
                assert digest(directory / "direct.bin") == EXPECTED
                run(f"{base}/via-http.bin", directory / "via-http.bin", "--connections", "4",
                    "--proxy", f"http://127.0.0.1:{ports['http']}")
                assert digest(directory / "via-http.bin") == EXPECTED
                target = f"http://localhost:{port}/via-socks.bin"
                run(target, directory / "via-socks.bin", "--connections", "4",
                    "--proxy", f"socks5h://user:secret@127.0.0.1:{ports['socks']}")
                assert digest(directory / "via-socks.bin") == EXPECTED
                run(target, directory / "bad-socks.bin",
                    "--proxy", f"socks5h://user:wrong@127.0.0.1:{ports['socks']}", success=False)
                run(f"{base}/bypass.bin", directory / "bypass.bin",
                    "--proxy", "http://127.0.0.1:9", "--proxy-bypass", "127.0.0.1")
                assert digest(directory / "bypass.bin") == EXPECTED
                routed = list(map(json.loads, proxy_log.read_text().splitlines()))
                assert any(r["kind"] == "http" and r["url"].endswith("/via-http.bin") for r in routed)
                socks = [r for r in routed if r["kind"] == "socks"]
                assert socks and all(r["host"] == "localhost" and r["address_type"] == "domain" for r in socks)
                assert not any("bypass" in r.get("url", "") for r in routed)
                assert not requests("/env.bin")
            finally:
                proxies.terminate()
                proxies.wait(timeout=5)
        finally:
            server.terminate()
            server.wait(timeout=5)
    print("http engine adoption, one-shot, replay, resume, no-clobber, proxy, stall recovery, range-corruption and throttle checks passed")


if __name__ == "__main__":
    main()
