-- ---------------------------------------------------------------------------
-- tack-server notes migration sidecar tables
-- (/Users/colin/.claude/plans/linked-roaming-rabbit.md)
--
-- clann-server keeps its own `/api/notes/*` surface backed by tack-server
-- (see src/tack_client.rs) rather than a clean pass-through, because tack's
-- `Note` schema has no column for `research_note.description`, and because
-- Clann's real per-tree visibility can't be evaluated by tack's ACL alone.
-- These two tables carry what tack itself can't:
--
--   - tack_note_meta: per-note sidecar for fields tack has no column for
--     (`description`) plus a transient field used only during backfill
--     (`legacy_folder_id`, nulled once Phase C repoints the note's real
--     tack folder). One row per *migrated* note, keyed by the tack note id
--     -- there's no row for a note created fresh in tack post-cutover
--     unless it also carries a description.
--
--   - tack_migration_state: the backfill's own idempotency marker,
--     deliberately kept as a *separate* table rather than a column added to
--     `research_note`/`research_folder` themselves, so the SurrealDB source
--     rows are never mutated by the migration -- they stay a fully intact,
--     byte-for-byte rollback target on their own (see the plan's "Rollback
--     strategy" section). `surreal_id` covers both notes and folders; kind
--     tells them apart.
-- ---------------------------------------------------------------------------

DEFINE TABLE IF NOT EXISTS tack_note_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS tack_note_id     ON TABLE tack_note_meta TYPE string;
DEFINE FIELD IF NOT EXISTS description      ON TABLE tack_note_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS legacy_folder_id ON TABLE tack_note_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at       ON TABLE tack_note_meta TYPE string VALUE <string>time::now() READONLY;
DEFINE FIELD IF NOT EXISTS updated_at       ON TABLE tack_note_meta TYPE string VALUE <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_tack_note_meta_note ON TABLE tack_note_meta COLUMNS tack_note_id UNIQUE;

DEFINE TABLE IF NOT EXISTS tack_migration_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS surreal_id  ON TABLE tack_migration_state TYPE string;
DEFINE FIELD IF NOT EXISTS kind        ON TABLE tack_migration_state TYPE string ASSERT $value IN ['folder', 'note'];
DEFINE FIELD IF NOT EXISTS tack_id     ON TABLE tack_migration_state TYPE string;
DEFINE FIELD IF NOT EXISTS migrated_at ON TABLE tack_migration_state TYPE string VALUE <string>time::now() READONLY;
DEFINE INDEX IF NOT EXISTS idx_migration_state_surreal ON TABLE tack_migration_state COLUMNS surreal_id, kind UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_migration_state_tack    ON TABLE tack_migration_state COLUMNS tack_id;
