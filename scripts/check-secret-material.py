#!/usr/bin/env python3
"""Reject tracked secret keys or official developer tool bundles.

This is defense in depth, not a replacement for GitHub secret scanning.
Offending content is never printed.
"""
from __future__ import annotations
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PEM = re.compile(rb"-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----")
TEXT_EXT = {".rs", ".cs", ".py", ".txt", ".md", ".json", ".toml",
            ".yaml", ".yml", ".xml", ".cmd", ".bat", ".ps1", ".sh", ".iss"}
FORBIDDEN_DIRS = {"evolve_tool_stage2", "official-stage2-tools"}
FORBIDDEN_FILES = {"private_key.pem", "id_rsa", "id_ed25519", "id_ecdsa",
                   "evolve_tool.exe"}
FORBIDDEN_SUFFIXES = {".p12", ".pfx", ".p8"}

def main() -> int:
    result = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT,
                            check=True, capture_output=True)
    failures = []
    for name in result.stdout.split(b"\0"):
        if not name:
            continue
        rel = pathlib.Path(name.decode("utf-8", errors="surrogateescape"))
        parts = {part.lower() for part in rel.parts}
        if (parts & FORBIDDEN_DIRS or rel.name.lower() in FORBIDDEN_FILES
                or rel.suffix.lower() in FORBIDDEN_SUFFIXES):
            failures.append(f"{rel}: restricted credential/tool")
            continue
        full = ROOT / rel
        if (full.is_file() and rel.suffix.lower() in TEXT_EXT
                and full.stat().st_size <= 16 * 1024 * 1024
                and PEM.search(full.read_bytes())):
            failures.append(f"{rel}: private PEM key marker")
    for failure in failures:
        print(f"ERROR: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("No tracked private signing credentials or official developer-tool bundles detected.")
    return 0

if __name__ == "__main__":
    sys.exit(main())
