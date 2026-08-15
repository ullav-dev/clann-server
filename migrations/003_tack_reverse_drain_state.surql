-- ---------------------------------------------------------------------------
-- Reverse-drain idempotency marker (/Users/colin/.claude/plans/linked-
-- roaming-rabbit.md, "Rollback strategy").
--
-- tack_reverse_drain_state is deliberately a SEPARATE table from
-- tack_migration_state (002_tack_migration_sidecar.surql), not a reused
-- kind value on it: tack_migration_state's unique index is
-- (surreal_id, kind), keyed by the SurrealDB source row; reverse-drain runs
-- in the opposite direction and needs the opposite key -- (tack_id, kind),
-- since a note/folder created fresh in tack after cutover has no
-- research_note/research_folder row to key against until reverse-drain
-- creates one. Keeping them separate also means a rollback rehearsal can
-- never accidentally collide with, or be mistaken for, the forward
-- backfill's own bookkeeping.
--
-- kind='folder' rows use tack_id as-is (a tack note_folders UUID always
-- reverse-maps to at most one research_folder, unlike the forward
-- direction's one-Clann-folder-to-many-tack-folders explosion -- see
-- src/bin/tack_backfill.rs's own doc comment on that).
-- ---------------------------------------------------------------------------

DEFINE TABLE IF NOT EXISTS tack_reverse_drain_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS tack_id     ON TABLE tack_reverse_drain_state TYPE string;
DEFINE FIELD IF NOT EXISTS kind        ON TABLE tack_reverse_drain_state TYPE string ASSERT $value IN ['folder', 'note'];
DEFINE FIELD IF NOT EXISTS surreal_id  ON TABLE tack_reverse_drain_state TYPE string;
DEFINE FIELD IF NOT EXISTS drained_at  ON TABLE tack_reverse_drain_state TYPE string VALUE <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_reverse_drain_tack ON TABLE tack_reverse_drain_state COLUMNS tack_id, kind UNIQUE;
