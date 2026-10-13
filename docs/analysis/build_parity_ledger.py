#!/usr/bin/env python3
"""Build the judge ledger: deterministic parsing of all behavior-contract tables.

Parses docs/idm_core_queue_contracts.md (N01-N48, Q01-Q21),
docs/idm_browser_behavior_contracts.md (B01-B62),
docs/idm_behavior_review.md (39 named contracts).
Outputs docs/parity_ledger.json and docs/parity_ledger.md.
--check exits non-zero if outputs are stale.
"""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path
from typing import Optional

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"

# Contract ID patterns
CORE_QUEUE_ID_RE = re.compile(r'^\*\*(N\d{2}|Q\d{2})\b')
BROWSER_ID_RE = re.compile(r'^\*\*(B\d{2})\b')
BEHAVIOR_REVIEW_ID_RE = re.compile(r'^`([a-z-]+)`')

# Status normalization
STATUS_MAP = {
    'implemented': 'implemented',
    'partial': 'partial',
    'different': 'different',
    'missing': 'missing',
    'gap': 'missing',
    'verified-subset': 'partial',
    'pending': 'missing',
    'platform-pending': 'missing',
    'unresolved': 'unknown',
    'intentional-substitute': 'different',
    'reference-only': 'unknown',
    'restricted': 'different',
    'independent': 'different',
}

# File existence cache
FILE_EXISTS_CACHE = {}


def file_exists_on_disk(path_str: str) -> bool:
    """Check if a file exists on disk, with caching."""
    if path_str in FILE_EXISTS_CACHE:
        return FILE_EXISTS_CACHE[path_str]
    # Normalize path relative to repo root
    p = ROOT / path_str
    exists = p.exists()
    FILE_EXISTS_CACHE[path_str] = exists
    return exists


def normalize_status(text: str) -> str:
    """Extract leading word and normalize to standard status."""
    if not text:
        return 'unknown'
    # Get first word, handling cases like "Partial:403" or "Not implemented:" or "Missing."
    # Extract leading alphabetic word before any punctuation/digit
    m = re.match(r'^([A-Za-z]+)', text.strip())
    if m:
        first_word = m.group(1).lower()
    else:
        first_word = text.strip().split()[0].lower().rstrip(':').rstrip('.')
    # Handle compound first words like "Not implemented"
    if first_word == 'not' and len(text.strip().split()) > 1:
        second = text.strip().split()[1].lower().rstrip(':').rstrip('.')
        if second in ('implemented', 'available', 'supported'):
            return 'missing'
    return STATUS_MAP.get(first_word, 'unknown')


def extract_test_refs(text: str) -> list[str]:
    """Extract test file references from text."""
    refs = []
    # Pattern for ../tests/... or tests/... or crates/*/tests/...
    patterns = [
        r'\.\./tests/[\w/.-]+\.(?:py|mjs|rs)',
        r'tests/[\w/.-]+\.(?:py|mjs|rs)',
        r'crates/[\w/-]+/tests/[\w/.-]+\.rs',
    ]
    for pat in patterns:
        for m in re.finditer(pat, text):
            ref = m.group(0)
            if ref not in refs and file_exists_on_disk(ref):
                refs.append(ref)
    # Also look for bare filenames mentioned in context of tests
    bare_pattern = r'(?:test|fixture)\s+(\w+\.(?:py|mjs|rs))'
    for m in re.finditer(bare_pattern, text, re.IGNORECASE):
        fname = m.group(1)
        # Try to find it in tests/integration or crates/*/tests
        for base in ['tests/integration', 'crates']:
            for p in ROOT.glob(f'{base}/**/{fname}'):
                rel = p.relative_to(ROOT)
                if str(rel) not in refs:
                    refs.append(str(rel))
    return refs


def parse_markdown_table(md_text: str, id_pattern: str) -> list[dict]:
    """Parse a markdown table with given ID pattern (e.g., N\\d{2} or Q\\d{2})."""
    contracts = []
    # Split into lines and find table rows
    lines = md_text.split('\n')
    in_table = False
    header_seen = False
    
    for line in lines:
        # Detect table start
        if '| ID / functionality' in line:
            in_table = True
            header_seen = True
            continue
        # Detect table end (empty line or next section)
        if in_table and (line.strip() == '' or line.startswith('## ')):
            in_table = False
            header_seen = False
            continue
        if not in_table or header_seen:
            if line.startswith('|---'):
                header_seen = False
            continue
        # Parse table row
        if line.startswith('|'):
            # Split by | but respect pipes in backticks
            # Simple approach: split and recombine if we have too many parts
            parts = line.split('|')
            # parts[0] is empty before first |, parts[-1] is after last |
            cells = [p.strip() for p in parts[1:-1]]
            if len(cells) >= 4:
                # First cell should contain **ID Title**
                first_cell = cells[0]
                m = re.match(r'\*\*(' + id_pattern + r')\b\s+(.+?)\*\*$', first_cell)
                if m:
                    cid = m.group(1)
                    title = m.group(2).strip()
                    evidence = cells[1] if len(cells) > 1 else ''
                    behavior = cells[2] if len(cells) > 2 else ''
                    target = cells[3] if len(cells) > 3 else ''
                    status = normalize_status(behavior)
                    test_refs = []
                    for cell in [evidence, behavior, target]:
                        test_refs.extend(extract_test_refs(cell))
                    contracts.append({
                        'id': cid,
                        'title': title,
                        'source_doc': 'idm_core_queue_contracts.md',
                        'current_behavior': behavior.strip(),
                        'status': status,
                        'test_refs': test_refs,
                    })
    return contracts


def parse_core_queue_contracts(md_text: str) -> list[dict]:
    """Parse the core queue contracts table (N01-N48, Q01-Q21)."""
    contracts = []
    contracts.extend(parse_markdown_table(md_text, r'N\d{2}'))
    contracts.extend(parse_markdown_table(md_text, r'Q\d{2}'))
    return contracts


def infer_browser_status(behavior: str) -> str:
    """Infer status from browser contract XDM state text."""
    b = behavior.lower()
    # Explicit positive indicators
    if any(w in b for w in ['implemented', 'verified', 'passes', 'works', 'fixture succeeds']):
        return 'implemented'
    # Partial indicators - has some implementation but not complete
    partial_indicators = ['partial', 'subset', 'exists', 'has ', 'only', 'basic ', 'limited ', 'requires ', 'stores ', 'records ', 'retains ', 'enumerates ', 'scans ', 'accepts ', 'opens ', 'logs ', 'supplies ', 'avoid ', 'can lose ', 'leaves ', 'cover ', 'framing ', 'validation', 'schema ', 'bound ']
    if any(w in b for w in partial_indicators):
        # But check for explicit missing/lacks
        if any(w in b for w in ['missing', 'lacks', 'does not', 'no ', 'absent', 'not implemented', 'cannot ', 'unable ']):
            # If it says "only" or "basic" or "limited" it's partial not missing
            if any(w in b for w in ['only', 'basic ', 'limited ', 'subset']):
                return 'partial'
            return 'missing'
        return 'partial'
    # Different indicators
    if any(w in b for w in ['different', 'differs', 'independent', 'rather than', 'instead of', 'deliberately ']):
        return 'different'
    # Missing indicators
    if any(w in b for w in ['missing', 'lacks', 'does not', 'no ', 'absent', 'not implemented', 'cannot ', 'unable ', 'unverified', 'pending']):
        return 'missing'
    # If it starts with "XDM " and describes functionality, assume partial
    if b.startswith('xdm ') and len(b) > 20:
        return 'partial'
    return 'unknown'


def parse_browser_contracts(md_text: str) -> list[dict]:
    """Parse the browser behavior contracts table (B01-B62)."""
    contracts = []
    # Tables have header: | ID | Original observable behavior and evidence | XDM state / exact difference | Required acceptance |
    tables = re.split(r'\n\| ID \| Original observable behavior', md_text)
    for table in tables[1:]:
        rows = re.findall(
            r'^\| (B\d{2}) \| ([^|]+) \| ([^|]+) \| ([^|]+) \|',
            table, re.MULTILINE
        )
        for cid, evidence, behavior, acceptance in rows:
            status = infer_browser_status(behavior)
            test_refs = []
            for cell in [evidence, behavior, acceptance]:
                test_refs.extend(extract_test_refs(cell))
            contracts.append({
                'id': cid,
                'title': evidence.strip().split('.')[0][:80],  # First sentence as title
                'source_doc': 'idm_browser_behavior_contracts.md',
                'current_behavior': behavior.strip(),
                'status': status,
                'test_refs': test_refs,
            })
    return contracts


def parse_behavior_review_contracts(md_text: str, json_data: dict) -> list[dict]:
    """Parse the behavior review contracts (39 named contracts)."""
    contracts = []
    for contract in json_data.get('contracts', []):
        cid = contract['id']
        title = contract['original_text_indicates'][:80]
        states = contract.get('xdm_family_states', {})
        behavior_summary = '; '.join(f'{k}: {v}' for k, v in states.items())
        # Determine overall status: if any implemented/verified-subset -> partial at minimum
        # Only missing if ALL are pending/platform-pending
        status_values = list(states.values())
        if 'implemented' in status_values:
            status = 'implemented'
        elif 'verified-subset' in status_values or 'partial' in status_values:
            status = 'partial'
        elif all(s in ('pending', 'platform-pending') for s in status_values):
            status = 'missing'
        else:
            status = 'unknown'
        test_refs = []
        test_refs.extend(extract_test_refs(contract.get('acceptance_scenario', '')))
        for ref in contract.get('source_references', []):
            test_refs.extend(extract_test_refs(ref))
        contracts.append({
            'id': cid,
            'title': title,
            'source_doc': 'idm_behavior_review.md',
            'current_behavior': behavior_summary,
            'status': status,
            'test_refs': list(set(test_refs)),
        })
    return contracts


def scan_tests_for_contract_ids() -> dict[str, list[str]]:
    """Scan all test files for contract IDs mentioned in comments/docstrings."""
    contract_to_tests = {}
    test_dirs = [
        ROOT / 'tests/integration',
        ROOT / 'crates',
    ]
    # Patterns for contract IDs
    id_patterns = [
        r'\b(N\d{2})\b',
        r'\b(Q\d{2})\b',
        r'\b(B\d{2})\b',
        r'\b(address-refresh|single-use-request|eager-confirmation|duplicate-policy|content-identity-recovery|unknown-length-completion|server-name-reconciliation|queue-independent-start|queue-order-and-retry|synchronization|recurrence-and-wake|quota-window|completion-precedence|completion-power|completion-program-and-scanner|category-retroactive|temporary-storage-cleanup|file-actions|remove-versus-delete|search-scopes|force-bypass-capture|miscapture-exceptions|all-links-and-clipboard|generated-batch|player-panel-policy|quality-and-track-catalog|live-stop-save|stream-assembly-fallback|minimum-media-size|proxy-selection|connection-profiles-and-host-caps|limiter-precedence|browser-platform-health|updater-integrity|tls-substitution|appearance-and-progress|grabber-auth-and-offline|grabber-project-recovery|playlist-link-expansion)\b',
    ]
    combined_pattern = re.compile('|'.join(f'({p})' for p in id_patterns))

    for test_dir in test_dirs:
        for ext in ['*.py', '*.mjs', '*.rs']:
            for test_file in test_dir.rglob(ext):
                try:
                    text = test_file.read_text()
                except Exception:
                    continue
                rel_path = test_file.relative_to(ROOT)
                for match in combined_pattern.finditer(text):
                    for group in match.groups():
                        if group:
                            cid = group
                            if cid not in contract_to_tests:
                                contract_to_tests[cid] = []
                            if str(rel_path) not in contract_to_tests[cid]:
                                contract_to_tests[cid].append(str(rel_path))
    return contract_to_tests


def build_ledger() -> dict:
    """Build the complete parity ledger."""
    # Read source documents
    core_queue_md = (DOCS / 'idm_core_queue_contracts.md').read_text()
    browser_md = (DOCS / 'idm_browser_behavior_contracts.md').read_text()
    behavior_review_md = (DOCS / 'idm_behavior_review.md').read_text()
    behavior_review_json = json.loads((DOCS / 'idm_behavior_review.json').read_text())

    # Parse contracts from each document
    all_contracts = []
    all_contracts.extend(parse_core_queue_contracts(core_queue_md))
    all_contracts.extend(parse_browser_contracts(browser_md))
    all_contracts.extend(parse_behavior_review_contracts(behavior_review_md, behavior_review_json))

    # Scan tests for contract ID mentions
    contract_to_tests = scan_tests_for_contract_ids()

    # Merge test references from scanning
    for contract in all_contracts:
        cid = contract['id']
        if cid in contract_to_tests:
            for test_ref in contract_to_tests[cid]:
                if test_ref not in contract['test_refs']:
                    contract['test_refs'].append(test_ref)

    # Compute summary counts per status per document
    summary = {}
    for contract in all_contracts:
        doc = contract['source_doc']
        status = contract['status']
        summary.setdefault(doc, {}).setdefault(status, 0)
        summary[doc][status] += 1

    # Sort contracts by source_doc then by ID
    def sort_key(c):
        doc_order = {
            'idm_core_queue_contracts.md': 0,
            'idm_browser_behavior_contracts.md': 1,
            'idm_behavior_review.md': 2,
        }
        return (doc_order.get(c['source_doc'], 99), c['id'])

    all_contracts.sort(key=sort_key)

    return {
        'schema_version': 1,
        'summary': summary,
        'contracts': all_contracts,
    }


def generate_markdown(ledger: dict) -> str:
    """Generate the markdown output."""
    lines = []
    lines.append('# XDM Parity Ledger')
    lines.append('')

    # Summary table
    lines.append('## Summary Counts per Document')
    lines.append('')
    lines.append('| Document | Implemented | Partial | Different | Missing | Unknown | Total |')
    lines.append('|---|---:|---:|---:|---:|---:|---:|')
    for doc, counts in ledger['summary'].items():
        impl = counts.get('implemented', 0)
        part = counts.get('partial', 0)
        diff = counts.get('different', 0)
        miss = counts.get('missing', 0)
        unk = counts.get('unknown', 0)
        total = impl + part + diff + miss + unk
        lines.append(f'| {doc} | {impl} | {part} | {diff} | {miss} | {unk} | {total} |')
    # Overall totals
    all_counts = {}
    for counts in ledger['summary'].values():
        for k, v in counts.items():
            all_counts[k] = all_counts.get(k, 0) + v
    impl = all_counts.get('implemented', 0)
    part = all_counts.get('partial', 0)
    diff = all_counts.get('different', 0)
    miss = all_counts.get('missing', 0)
    unk = all_counts.get('unknown', 0)
    total = impl + part + diff + miss + unk
    lines.append(f'| **Total** | **{impl}** | **{part}** | **{diff}** | **{miss}** | **{unk}** | **{total}** |')
    lines.append('')

    # Contract detail table
    lines.append('## Contract Details')
    lines.append('')
    lines.append('| ID | Title | Status | Current XDM Behavior | Tests |')
    lines.append('|---|---|---|---|---|')
    for c in ledger['contracts']:
        tests_str = ', '.join(c['test_refs']) if c['test_refs'] else '—'
        # Truncate long fields for table readability
        title = c['title'][:60] + ('…' if len(c['title']) > 60 else '')
        behavior = c['current_behavior'][:80] + ('…' if len(c['current_behavior']) > 80 else '')
        lines.append(f"| {c['id']} | {title} | {c['status']} | {behavior} | {tests_str} |")
    lines.append('')

    # Work queue section
    lines.append('## Work Queue')
    lines.append('')
    lines.append('### Missing/Partial contracts with NO referenced test (highest priority)')
    lines.append('')
    lines.append('| ID | Title | Status | Source Document |')
    lines.append('|---|---|---|---|')
    no_test = [c for c in ledger['contracts'] if c['status'] in ('missing', 'partial') and not c['test_refs']]
    for c in no_test:
        title = c['title'][:60] + ('…' if len(c['title']) > 60 else '')
        lines.append(f"| {c['id']} | {title} | {c['status']} | {c['source_doc']} |")
    lines.append('')
    lines.append(f'**Count: {len(no_test)}**')
    lines.append('')

    lines.append('### Partial contracts WITH tests (next priority)')
    lines.append('')
    lines.append('| ID | Title | Status | Tests | Source Document |')
    lines.append('|---|---|---|---|---|')
    partial_with_tests = [c for c in ledger['contracts'] if c['status'] == 'partial' and c['test_refs']]
    for c in partial_with_tests:
        title = c['title'][:60] + ('…' if len(c['title']) > 60 else '')
        tests_str = ', '.join(c['test_refs'])
        lines.append(f"| {c['id']} | {title} | {c['status']} | {tests_str} | {c['source_doc']} |")
    lines.append('')
    lines.append(f'**Count: {len(partial_with_tests)}**')
    lines.append('')

    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true', help='Exit non-zero if outputs are stale')
    args = parser.parse_args()

    ledger = build_ledger()
    json_out = DOCS / 'parity_ledger.json'
    md_out = DOCS / 'parity_ledger.md'

    json_data = json.dumps(ledger, indent=2, ensure_ascii=False) + '\n'
    md_data = generate_markdown(ledger)

    if args.check:
        stale = False
        if json_out.exists():
            if json_out.read_text() != json_data:
                print(f'Stale: {json_out}')
                stale = True
        else:
            print(f'Missing: {json_out}')
            stale = True
        if md_out.exists():
            if md_out.read_text() != md_data:
                print(f'Stale: {md_out}')
                stale = True
        else:
            print(f'Missing: {md_out}')
            stale = True
        if stale:
            return 1
        print('Parity ledger is up to date.')
        return 0
    else:
        json_out.write_text(json_data)
        md_out.write_text(md_data)
        print(f'Generated {json_out} and {md_out}')
        # Print summary counts
        all_counts = {}
        for counts in ledger['summary'].values():
            for k, v in counts.items():
                all_counts[k] = all_counts.get(k, 0) + v
        print(f"Summary: implemented={all_counts.get('implemented',0)} partial={all_counts.get('partial',0)} different={all_counts.get('different',0)} missing={all_counts.get('missing',0)} unknown={all_counts.get('unknown',0)} total={sum(all_counts.values())}")
        return 0


if __name__ == '__main__':
    exit(main())