#!/usr/bin/env python3
"""Produce a certification request set for the read-only evidence family.

This tool NEVER certifies anything by itself and never executes module code.
Certification is a human reviewer decision recorded through the authenticated
runtime API by a user holding the Reviewer role. This script only computes the
exact module IDs and SHA-256 content hashes a reviewer must approve, and prints
the API calls that would record that decision.

Usage:
    python3 scripts/certify-evidence.py                  # print the review sheet
    python3 scripts/certify-evidence.py --json out.json  # also write JSON
    python3 scripts/certify-evidence.py --curl           # print curl commands

The printed hashes are the same values `PluginExecutor::plugin_sha256` computes
(SHA-256 of the file bytes), so a certification recorded from this output will
match the runtime's check. Editing a module changes its hash and invalidates any
prior certification.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

FAMILY_DIR = Path("plugins/evidence_readonly")
REQUIRED_FEATURE = "forensics"

# The manifest reader in janus-runtime/src/plugin.rs accepts a fully commented
# --[{ ... }] block. Mirror it here so IDs come from the manifest, not the path.
def read_manifest(source: str) -> dict | None:
    lines = source.split("\n")
    for index, line in enumerate(lines):
        stripped = line.lstrip()
        if not stripped.startswith("--["):
            continue
        after = stripped[3:]
        if not after.startswith("{"):
            continue
        if "}]" in after:
            body = after[: after.find("}]") + 1]
        else:
            collected = [after]
            for follow in lines[index + 1 :]:
                trimmed = follow.lstrip()
                content = trimmed[2:] if trimmed.startswith("--") else trimmed
                if "}]" in content:
                    collected.append(content[: content.find("}]") + 1])
                    break
                collected.append(content)
            body = "\n".join(collected)
        try:
            return json.loads(body)
        except json.JSONDecodeError:
            return None
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", metavar="FILE", help="write the review sheet as JSON")
    parser.add_argument("--curl", action="store_true", help="print curl commands")
    parser.add_argument(
        "--runtime-url",
        default="http://127.0.0.1:8080",
        help="runtime base URL used in printed curl commands",
    )
    args = parser.parse_args()

    if not FAMILY_DIR.is_dir():
        print(f"error: {FAMILY_DIR} not found; run from the repository root", file=sys.stderr)
        return 1

    entries = []
    for path in sorted(FAMILY_DIR.glob("*.lua")):
        data = path.read_bytes()
        manifest = read_manifest(data.decode("utf-8"))
        if manifest is None:
            print(f"error: {path} has no parseable manifest", file=sys.stderr)
            return 1
        entries.append(
            {
                "module_id": manifest["id"],
                "module_sha256": hashlib.sha256(data).hexdigest(),
                "path": path.as_posix(),
                "name": manifest.get("name", ""),
                "description": manifest.get("description", ""),
                "required_feature": REQUIRED_FEATURE,
                "requested_status": "production_approved",
                "bytes": len(data),
            }
        )

    if not entries:
        print(f"error: no modules found in {FAMILY_DIR}", file=sys.stderr)
        return 1

    print("READ-ONLY EVIDENCE FAMILY — REVIEW SHEET")
    print("=" * 72)
    print("These modules perform real, read-only filesystem operations confined to")
    print("the evidence paths a customer approved for an active engagement.")
    print("Nothing is certified by running this script. A Reviewer must record the")
    print("decision through POST /modules/certifications after reviewing the code.")
    print("=" * 72)
    for entry in entries:
        print(f"\n  {entry['module_id']}")
        print(f"    file   : {entry['path']} ({entry['bytes']} bytes)")
        print(f"    sha256 : {entry['module_sha256']}")
        print(f"    feature: {entry['required_feature']}")
        print(f"    purpose: {entry['description']}")

    print("\n" + "=" * 72)
    print("Before approving, confirm for each module:")
    print("  1. It only calls janus.evidence.* and janus.log / print.")
    print("  2. It performs no write, delete, shell, ADB, or network operation.")
    print("  3. Its arguments cannot widen the approved evidence scope.")
    print("  4. cargo test --workspace passes, including the sandbox escape tests.")
    print("=" * 72)

    if args.curl:
        print("\n# Record the decision as a Reviewer. TOKEN must be a Reviewer session.")
        print("# Review the code first; these commands grant production execution.")
        for entry in entries:
            payload = {
                "module_id": entry["module_id"],
                "module_sha256": entry["module_sha256"],
                "status": "production_approved",
                "required_feature": entry["required_feature"],
                "notes": "Read-only evidence collection; reviewed against the audit checklist.",
            }
            print(
                f"curl -sS -X POST '{args.runtime_url}/modules/certifications' \\\n"
                f"  -H \"Authorization: Bearer $JANUS_REVIEWER_TOKEN\" \\\n"
                f"  -H 'Content-Type: application/json' \\\n"
                f"  -d '{json.dumps(payload)}'"
            )

    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(
            json.dumps(
                {
                    "family": "evidence_readonly",
                    "required_feature": REQUIRED_FEATURE,
                    "certified_by_this_tool": False,
                    "modules": entries,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        print(f"\nReview sheet written to {out}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
