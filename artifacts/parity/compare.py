#!/usr/bin/env python3
"""Compare two JSON startup-list files from MichStartupMaster."""
import json, sys

def load(path):
    with open(path) as f:
        data = json.load(f)
    assert isinstance(data, list), f"{path}: root is not an array"
    return data

cs = load(sys.argv[1])
rust = load(sys.argv[2])

# --- basic counts ---
print("=" * 60)
print("PARITY REPORT")
print("=" * 60)
print(f"C# count : {len(cs)}")
print(f"Rust count: {len(rust)}")
print()

# --- duplicate-id check per file ---
from collections import Counter
cs_ids = [e["id"] for e in cs]
rust_ids = [e["id"] for e in rust]

cs_dupes = Counter(cs_ids)
rust_dupes = Counter(rust_ids)
cs_dupe_count = sum(1 for c in cs_dupes.values() if c > 1)
rust_dupe_count = sum(1 for c in rust_dupes.values() if c > 1)
print(f"C# dup-ID entries : {cs_dupe_count}")
print(f"Rust dup-ID entries: {rust_dupe_count}")
if cs_dupe_count:
    print(f"  C# duplicated IDs: {[i for i,c in cs_dupes.items() if c > 1][:5]}...")
if rust_dupe_count:
    print(f"  Rust duplicated IDs: {[i for i,c in rust_dupes.items() if c > 1][:5]}...")
print()

# --- source histogram diff ---
cs_src = Counter(e.get("source","?") for e in cs)
rust_src = Counter(e.get("source","?") for e in rust)
all_sources = sorted(set(list(cs_src.keys()) + list(rust_src.keys())))
print("SOURCE HISTOGRAM:")
print(f"{'Source':<40} {'C#':>8} {'Rust':>8} {'Diff':>8}")
for s in all_sources:
    cn = cs_src.get(s, 0)
    rn = rust_src.get(s, 0)
    d = cn - rn
    bar_c = "#" * cn
    bar_r = "#" * rn
    print(f"{s:<40} {cn:>8} {rn:>8} {d:+>8}")
print()

# --- ids-only-in-cs ---
cs_id_set = set(cs_ids)
rust_id_set = set(rust_ids)
only_in_cs = sorted(cs_id_set - rust_id_set)[:80]
only_in_rust = sorted(rust_id_set - cs_id_set)[:80]
print(f"IDs only in C# (first {len(only_in_cs)}) : {only_in_cs}")
print(f"IDs only in Rust (first {len(only_in_rust)}): {only_in_rust}")
print()

# --- for first 20 cs-only ids also show source+name ---
show = min(20, len(only_in_cs))
if show:
    print("FIRST 20 CS-ONLY details (id | source | name):")
    for eid in only_in_cs[:show]:
        rec = next((e for e in cs if e["id"] == eid), None)
        src = rec.get("source","?") if rec else "?"
        nm = rec.get("name","?") if rec else "?"
        print(f"  {eid} | {src} | {nm}")
print()
print("=" * 60)
