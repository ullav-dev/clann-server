#!/usr/bin/env python3
"""Analyzes the JSON dump produced by census_notes_migration.sh -- computes
the Phase 0 gate numbers for the tack-server notes migration plan
(/Users/colin/.claude/plans/linked-roaming-rabbit.md).

Read-only: this script only reads the JSON file passed to it. It never
connects to any database itself.

Usage:
    ./census_notes_migration.sh > census_output.json
    ./census_notes_migration.py census_output.json
"""

import json
import sys
from collections import defaultdict


def main() -> None:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <census_output.json>", file=sys.stderr)
        sys.exit(1)

    with open(sys.argv[1]) as f:
        raw = json.load(f)

    # SurrealDB's HTTP /sql endpoint returns a list of per-statement result
    # objects, each with a "result" key -- census_notes_migration.sh issued
    # two statements per curl call, but we made two separate calls, so each
    # top-level key here is itself that per-call response list.
    trees_response = raw["trees"]
    notes_response = raw["notes"]

    def unwrap(resp):
        # Handles both "the driver already unwrapped it to a plain list"
        # and "raw HTTP response list-of-statement-results" shapes, since
        # which one you get depends on how `sql()` was invoked.
        if isinstance(resp, list) and resp and isinstance(resp[0], dict) and "result" in resp[0]:
            return resp[0]["result"]
        return resp

    trees = unwrap(trees_response)
    notes = unwrap(notes_response)

    # name -> team_id (None if the tree has no team)
    tree_team = {t["name"]: t.get("team_id") for t in trees}
    valid_tree_names = set(tree_team.keys())

    total_notes = len(notes)
    unresolvable_slug_notes = []
    empty_trees_shared_notes = []
    multi_team_notes = []
    shared_no_team_notes = []

    for n in notes:
        note_id = n["id"]
        trees_field = n.get("trees") or []
        is_shared = bool(n.get("is_shared"))

        unresolvable = [t for t in trees_field if t not in valid_tree_names]
        if unresolvable:
            unresolvable_slug_notes.append({"id": note_id, "title": n.get("title"), "unresolvable_slugs": unresolvable})

        if not trees_field and is_shared:
            empty_trees_shared_notes.append({"id": note_id, "title": n.get("title")})

        resolved_team_ids = {tree_team[t] for t in trees_field if t in valid_tree_names}
        resolved_team_ids_non_null = {t for t in resolved_team_ids if t is not None}
        if len(resolved_team_ids_non_null) > 1:
            multi_team_notes.append({"id": note_id, "title": n.get("title"), "team_ids": sorted(resolved_team_ids_non_null)})

        if is_shared and trees_field and not resolved_team_ids_non_null:
            # Every resolved tree (if any) has no team -- shared with
            # nowhere to actually share to.
            resolvable_trees = [t for t in trees_field if t in valid_tree_names]
            if resolvable_trees:
                shared_no_team_notes.append({"id": note_id, "title": n.get("title")})

    print(f"Total top-level research_note rows examined: {total_notes}")
    print(f"Total family_tree rows: {len(trees)}")
    print()
    print(f"Notes with >=1 unresolvable tree slug: {len(unresolvable_slug_notes)}")
    print(f"Notes with trees=[] but is_shared=true: {len(empty_trees_shared_notes)}")
    print(f"Notes whose trees resolve to >1 distinct team: {len(multi_team_notes)}")
    print(f"Notes is_shared=true where every resolved tree has no team: {len(shared_no_team_notes)}")
    print()
    print("Every note counted above (for the Phase 0 review -- do not act on any of")
    print("this without a human reading it; the migration's own policy is to migrate")
    print("all of these as visibility=private and log them for individual follow-up):")
    print()
    detail = {
        "unresolvable_slug_notes": unresolvable_slug_notes,
        "empty_trees_shared_notes": empty_trees_shared_notes,
        "multi_team_notes": multi_team_notes,
        "shared_no_team_notes": shared_no_team_notes,
    }
    print(json.dumps(detail, indent=2))


if __name__ == "__main__":
    main()
