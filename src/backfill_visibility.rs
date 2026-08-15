//! Visibility policy for the Clann -> tack-server notes migration's Phase 1
//! backfill (/Users/colin/.claude/plans/linked-roaming-rabbit.md). One pure
//! function (`decide`), one call site (the per-note loop in
//! `src/bin/tack_backfill.rs`) -- kept separate from `src/backfill.rs`'s
//! resolution layer deliberately, so the plan's still-unsigned-off
//! default-private-on-ambiguity policy lives in exactly one swappable place,
//! not threaded through the loop itself.
//!
//! `research_note.is_shared`/`trees[]` don't map onto tack's
//! `(team_id, Visibility)` cleanly -- a note may reference zero trees, a
//! tree with no team, or trees spanning more than one team (the Phase 0
//! census script's own four flagged categories). This module is the single
//! place that turns that ambiguity into one of three concrete outcomes,
//! per the plan's own rule: **never resolve ambiguity toward wider
//! exposure** -- an ambiguous note still migrates (so it isn't lost), but
//! always as `Visibility::Private`, and always flagged for a human to
//! review individually. Only "no team can be determined at all" is a hard
//! skip -- tack's own `CreateNoteRequest.team_id` is a required `Uuid`
//! (verified directly against `handlers/notes.rs`), so there is no way to
//! migrate a note tack itself would reject for having nowhere to file it.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Private,
    Team,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MigrationTarget {
    pub team_id: String,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityReason {
    /// The note's resolved trees span more than one distinct team --
    /// there's no principled way to pick "the" team, so the
    /// lexicographically-first one is used (deterministic and reproducible
    /// across dry-run/real runs) and the note is flagged for a human to
    /// confirm or re-file.
    MultiTeam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "reason")]
pub enum SkipReason {
    /// The note references no tree at all (`trees == []`).
    NoTrees,
    /// Every tree slug the note references failed to resolve (see
    /// `backfill::resolve_note_trees`) -- functionally the same as
    /// `NoTrees` from tack's point of view (no team to file under), kept
    /// as a distinct reason code for the skip-list report.
    AllTreeSlugsUnresolvable,
    /// At least one referenced tree resolved, but none of them belong to a
    /// team (`family_tree.team_id IS NULL` for all of them) -- the Phase 0
    /// census's "shared, no team" category, generalized to apply
    /// regardless of `is_shared`.
    NoTeamOnAnyResolvedTree,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Decision {
    /// Exactly one distinct team resolved -- the clean, expected case.
    Migrate(MigrationTarget),
    /// A team was determined, but only by breaking a tie the plan itself
    /// says must never resolve toward wider exposure -- always
    /// `Visibility::Private` regardless of `is_shared`, and always
    /// reported so a human reviews it individually (per the plan's
    /// "skip list [and flagged migrations] are a go/no-go gate
    /// themselves").
    MigrateAmbiguous(MigrationTarget, AmbiguityReason),
    /// No team can be determined at all -- tack has nowhere to file this
    /// note, so it cannot be migrated as-is. Not a data-loss silent drop:
    /// every skip is logged for individual review, per the plan.
    Skip(SkipReason),
}

/// `is_shared`: the note's own `research_note.is_shared` flag.
/// `resolved_team_ids`: the **distinct, non-null** `team_id`s of every tree
/// the note references that resolved successfully (via
/// `backfill::resolve_note_trees`) -- trees that failed to resolve, or that
/// resolved but have no team, are simply absent from this set; this
/// function doesn't need to know *why* a tree didn't contribute a team,
/// only whether any did. `had_any_slugs`/`had_any_resolved_trees`
/// distinguish the three skip reasons for reporting.
pub fn decide(
    is_shared: bool,
    resolved_team_ids: &BTreeSet<String>,
    had_any_slugs: bool,
    had_any_resolved_trees: bool,
) -> Decision {
    if !had_any_slugs {
        return Decision::Skip(SkipReason::NoTrees);
    }
    if !had_any_resolved_trees {
        return Decision::Skip(SkipReason::AllTreeSlugsUnresolvable);
    }
    match resolved_team_ids.len() {
        0 => Decision::Skip(SkipReason::NoTeamOnAnyResolvedTree),
        1 => {
            let team_id = resolved_team_ids.iter().next().unwrap().clone();
            let visibility = if is_shared { Visibility::Team } else { Visibility::Private };
            Decision::Migrate(MigrationTarget { team_id, visibility })
        }
        _ => {
            // BTreeSet is already sorted -- first() is the deterministic,
            // reproducible tie-break.
            let team_id = resolved_team_ids.iter().next().unwrap().clone();
            Decision::MigrateAmbiguous(
                MigrationTarget { team_id, visibility: Visibility::Private },
                AmbiguityReason::MultiTeam,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn single_team_shared_note_migrates_as_team_visible() {
        let d = decide(true, &set(&["team-a"]), true, true);
        assert_eq!(
            d,
            Decision::Migrate(MigrationTarget { team_id: "team-a".into(), visibility: Visibility::Team })
        );
    }

    #[test]
    fn single_team_private_note_migrates_as_private() {
        let d = decide(false, &set(&["team-a"]), true, true);
        assert_eq!(
            d,
            Decision::Migrate(MigrationTarget { team_id: "team-a".into(), visibility: Visibility::Private })
        );
    }

    #[test]
    fn empty_trees_is_a_skip_regardless_of_is_shared() {
        // Phase 0 census category: trees=[] but is_shared=true.
        assert_eq!(decide(true, &set(&[]), false, false), Decision::Skip(SkipReason::NoTrees));
        assert_eq!(decide(false, &set(&[]), false, false), Decision::Skip(SkipReason::NoTrees));
    }

    #[test]
    fn all_slugs_unresolvable_is_its_own_skip_reason() {
        let d = decide(true, &set(&[]), true, false);
        assert_eq!(d, Decision::Skip(SkipReason::AllTreeSlugsUnresolvable));
    }

    #[test]
    fn resolved_tree_with_no_team_is_a_skip_not_a_guess() {
        // Phase 0 census category: shared, resolved tree(s), no team.
        let d = decide(true, &set(&[]), true, true);
        assert_eq!(d, Decision::Skip(SkipReason::NoTeamOnAnyResolvedTree));
    }

    #[test]
    fn multi_team_never_resolves_to_shared_even_if_is_shared_true() {
        // Phase 0 census category: resolves to >1 distinct team.
        let d = decide(true, &set(&["team-b", "team-a"]), true, true);
        assert_eq!(
            d,
            Decision::MigrateAmbiguous(
                MigrationTarget { team_id: "team-a".into(), visibility: Visibility::Private },
                AmbiguityReason::MultiTeam
            )
        );
    }

    #[test]
    fn multi_team_tie_break_is_deterministic_across_runs() {
        let a = decide(false, &set(&["zzz", "aaa", "mmm"]), true, true);
        let b = decide(false, &set(&["mmm", "zzz", "aaa"]), true, true);
        assert_eq!(a, b);
        assert_eq!(
            a,
            Decision::MigrateAmbiguous(
                MigrationTarget { team_id: "aaa".into(), visibility: Visibility::Private },
                AmbiguityReason::MultiTeam
            )
        );
    }
}
