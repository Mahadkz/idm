"""Review 7: the CLI refuses passwords inside proxy URLs (argv is world-readable) and says where to put them."""
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from auth_harness import ROOT, Daemon


def main():
    cli = str(ROOT / "target/debug/xdm-cli")
    usage = subprocess.run([cli, "--help"], capture_output=True, text=True).stderr
    assert "USER:PASS@" not in usage and "proxy-login" in usage, usage
    direct = subprocess.run([cli, "http://127.0.0.1:1/f", "/tmp/xdm-never.bin", "--proxy", "http://u:pw@127.0.0.1:9"],
                            capture_output=True, text=True)
    assert direct.returncode != 0 and "proxy-login" in direct.stderr and "pw" not in direct.stderr.replace("password", ""), direct.stderr
    daemon = Daemon("xdm-cliproxy-")
    try:
        refused = subprocess.run([cli, "daemon", "proxy", "manual", "http://u:pw@127.0.0.1:9"],
                                 env=daemon.env, capture_output=True, text=True)
        assert refused.returncode != 0 and "proxy-login" in refused.stderr, refused
        assert daemon.cli("proxy", "get")["proxy"] == {"mode": "system"}
        assert daemon.cli("proxy", "manual", "http://127.0.0.1:9")["ok"]
        print(json.dumps({"result": "passed", "cases": ["--proxy userinfo refused", "daemon proxy manual userinfo refused", "usage text"]}))
    finally:
        daemon.close()


main()
