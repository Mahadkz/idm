"""Check cross-artifact identity coverage; these checks do not verify runtime parity."""
import hashlib
import json
import re
from collections import Counter
from pathlib import Path

from build_feature_coverage import build as coverage, markdown as coverage_markdown
from build_behavior_review import build as behavior, markdown as behavior_markdown
from build_control_review import generate as controls

ROOT = Path(__file__).resolve().parents[2]


def current(path, data):
    assert (ROOT / path).read_text() == data, f"Stale mapping artifact: {path}"


def check():
    ledger = coverage()
    review = behavior()
    control_review, control_md = controls()
    for path, item in [("docs/feature_coverage.json", ledger),
                       ("docs/idm_behavior_review.json", review),
                       ("docs/idm_control_review.json", control_review)]:
        current(path, json.dumps(item, indent=2, ensure_ascii=False) + "\n")
    current("docs/feature_coverage.md", coverage_markdown(ledger))
    current("docs/idm_behavior_review.md", behavior_markdown(review))
    current("docs/idm_control_review.md", control_md)

    source = json.loads((ROOT / ledger["source"]).read_bytes())
    expected_controls = {f"D{d['resource_id']}/C{i}": c
                         for d in source["dialogs"] for i, c in enumerate(d["controls"])}
    control_rows = {row["key"]: row for d in control_review["dialogs"] for row in d["controls"]}
    assert set(expected_controls) == set(control_rows) == {c["source_key"] for c in ledger["controls"]}
    for key, original in expected_controls.items():
        row = control_rows[key]
        for output, field in [("source_id", "id"), ("source_class", "class"),
                              ("source_style", "style"), ("source_ex_style", "ex_style"),
                              ("source_bounds_dlu", "bounds_dlu"), ("source_caption", "title")]:
            assert row[output] == original[field], (key, field)
    assert {row["source_key"] for row in review["references"]} == {row["source_key"] for row in ledger["language_references"]}

    browser = json.loads((ROOT / "docs/analysis/idm_browser_source_inventory.json").read_bytes())
    observed_counts = Counter(item["kind"] for file in browser["files"] for item in file["items"])
    assert dict(observed_counts) == browser["counts"]
    # Temporary proprietary inputs may disappear; when present, verify provenance.
    checked_sources = 0
    for file in browser["files"]:
        path = Path(browser["source_directory"]) / file["file"]
        if path.exists():
            assert hashlib.sha256(path.read_bytes()).hexdigest() == file["sha256"], path
            checked_sources += 1
    browser_doc = (ROOT / "docs/idm_browser_behavior_contracts.md").read_text()
    browser_ids = re.findall(r"^\| B(\d+) \|", browser_doc, re.M)
    assert list(map(int, browser_ids)) == list(range(1, browser["contracts"] + 1))
    core_doc = (ROOT / "docs/idm_core_queue_contracts.md").read_text()
    core_ids = re.findall(r"^\| \*\*([NQ]\d+)\b", core_doc, re.M)
    assert core_ids == [f"N{i:02d}" for i in range(1, 49)] + [f"Q{i:02d}" for i in range(1, 22)]
    for document in [browser_doc, core_doc]:
        for _, target in re.findall(r"\[([^\]]+)\]\(([^)]+)\)", document):
            if "://" not in target:
                assert (ROOT / "docs" / target.split("#")[0]).exists(), target
    return {"dialogs": len(source["dialogs"]), "controls": len(expected_controls),
            "static_menu_commands": ledger["counts"]["menu_commands"],
            "reviewed_language_entries": len(review["references"]),
            "pe_string_references": ledger["counts"]["pe_string_entries"],
            "detailed_contracts": len(review["contracts"]) + len(browser_ids) + len(core_ids),
            "unresolved_control_semantics": control_review["counts"]["unresolved_semantics"],
            "unresolved_static_text_associations": ledger["counts"]["unresolved_dynamic_references"],
            "temporary_browser_sources_hash_checked": checked_sources,
            "runtime_parity_verified_by_this_check": False}


if __name__ == "__main__":
    print(json.dumps(check(), indent=2))
