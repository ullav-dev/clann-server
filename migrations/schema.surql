DEFINE TABLE IF NOT EXISTS family_tree SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name         ON TABLE family_tree TYPE string;
DEFINE FIELD IF NOT EXISTS display_name ON TABLE family_tree TYPE string;
DEFINE FIELD IF NOT EXISTS owner        ON TABLE family_tree TYPE string;
DEFINE FIELD IF NOT EXISTS is_primary   ON TABLE family_tree TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS team_id      ON TABLE family_tree TYPE option<string>;
DEFINE FIELD IF NOT EXISTS image_path   ON TABLE family_tree TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_tree_name  ON TABLE family_tree COLUMNS name UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_tree_owner ON TABLE family_tree COLUMNS owner;

-- Canonical person: verifiable identity record (birth certificate ground truth).
-- Enrichment fields (biography, images, etc.) live on person_proxy.
-- DO NOT add display preferences here; those belong on person_proxy.
DEFINE TABLE IF NOT EXISTS person SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS family_name   ON TABLE person TYPE string;
DEFINE FIELD IF NOT EXISTS first_name    ON TABLE person TYPE string;
DEFINE FIELD IF NOT EXISTS middle_name   ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS sex           ON TABLE person TYPE string ASSERT $value IN ["Male", "Female"];
DEFINE FIELD IF NOT EXISTS date_of_birth  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS place_of_birth ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS date_of_death  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS place_of_death ON TABLE person TYPE option<string>;
-- Official document references (strongest verifiable identity)
DEFINE FIELD IF NOT EXISTS birth_cert_authority ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS birth_cert_number    ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_by    ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at    ON TABLE person TYPE option<string> DEFAULT <string>time::now();
-- Legacy fields kept until Phase 4 cleanup (new records will not use these)
DEFINE FIELD IF NOT EXISTS image_path       ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS life_image_path  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS image_bytes      ON TABLE person TYPE option<int>;
DEFINE FIELD IF NOT EXISTS life_image_bytes ON TABLE person TYPE option<int>;
DEFINE FIELD IF NOT EXISTS nickname      ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS username      ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS email         ON TABLE person TYPE option<string>;
DEFINE FIELD verified                    ON TABLE person TYPE bool DEFAULT false VALUE $value OR false;
DEFINE FIELD biography                   ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS tree          ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS trees         ON TABLE person TYPE array<string> DEFAULT [];
-- Unique only when BOTH cert fields are set (SurrealDB excludes NONE from UNIQUE index)
DEFINE INDEX IF NOT EXISTS unique_birth_cert
  ON TABLE person COLUMNS birth_cert_authority, birth_cert_number UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_created_by  ON TABLE person COLUMNS created_by;
REMOVE INDEX IF EXISTS idx_person_tree ON TABLE person;
DEFINE INDEX IF NOT EXISTS idx_person_trees ON TABLE person COLUMNS trees;

-- Person proxy: tree-local view of a canonical person.
-- One proxy per (person, tree) pair. Relationships are proxy→proxy edges.
DEFINE TABLE IF NOT EXISTS person_proxy SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS person_id             ON TABLE person_proxy TYPE record<person>;
DEFINE FIELD IF NOT EXISTS tree                  ON TABLE person_proxy TYPE string;
-- Optional display-name overrides (null = show canonical name)
DEFINE FIELD IF NOT EXISTS preferred_family_name ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS preferred_first_name  ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS preferred_middle_name ON TABLE person_proxy TYPE option<string>;
-- Tree-local enrichment
DEFINE FIELD IF NOT EXISTS nickname         ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS username         ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS email            ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD verified                       ON TABLE person_proxy TYPE bool DEFAULT false VALUE $value OR false;
DEFINE FIELD IF NOT EXISTS biography        ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS image_path       ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS life_image_path  ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS image_bytes      ON TABLE person_proxy TYPE option<int>;
DEFINE FIELD IF NOT EXISTS life_image_bytes ON TABLE person_proxy TYPE option<int>;
DEFINE FIELD IF NOT EXISTS is_private       ON TABLE person_proxy TYPE bool DEFAULT false VALUE $value OR false;
DEFINE FIELD IF NOT EXISTS created_by       ON TABLE person_proxy TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at       ON TABLE person_proxy TYPE option<string> DEFAULT <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_proxy_person  ON TABLE person_proxy COLUMNS person_id;
DEFINE INDEX IF NOT EXISTS idx_proxy_tree    ON TABLE person_proxy COLUMNS tree;
DEFINE INDEX IF NOT EXISTS idx_proxy_creator ON TABLE person_proxy COLUMNS created_by;
DEFINE INDEX IF NOT EXISTS unique_proxy_canonical_tree
  ON TABLE person_proxy COLUMNS person_id, tree UNIQUE;

-- Life Events: tree-local by default; is_canonical=true promotes to shared visibility.
-- person_proxy_id is the primary link; person_id kept as option<> for legacy data.
DEFINE TABLE IF NOT EXISTS life_event SCHEMAFULL;
-- person_id: legacy field (was required in <v26.3); now optional for backward compat
DEFINE FIELD OVERWRITE person_id      ON TABLE life_event TYPE option<record<person>>;
DEFINE FIELD IF NOT EXISTS person_proxy_id     ON TABLE life_event TYPE option<record<person_proxy>>;
DEFINE FIELD IF NOT EXISTS is_canonical        ON TABLE life_event TYPE bool DEFAULT false VALUE $value OR false;
DEFINE FIELD IF NOT EXISTS contributed_by_tree ON TABLE life_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS name         ON TABLE life_event TYPE string;
DEFINE FIELD IF NOT EXISTS date         ON TABLE life_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS event_type   ON TABLE life_event TYPE string;
DEFINE FIELD IF NOT EXISTS description  ON TABLE life_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS story        ON TABLE life_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS verified     ON TABLE life_event TYPE bool DEFAULT false VALUE $value OR false;
DEFINE FIELD IF NOT EXISTS source_link  ON TABLE life_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS source_image ON TABLE life_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS source_doc   ON TABLE life_event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_by   ON TABLE life_event TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_life_event_person  ON TABLE life_event COLUMNS person_id;
DEFINE INDEX IF NOT EXISTS idx_life_event_proxy   ON TABLE life_event COLUMNS person_proxy_id;
DEFINE INDEX IF NOT EXISTS idx_life_event_creator ON TABLE life_event COLUMNS created_by;
DEFINE INDEX IF NOT EXISTS idx_life_event_canonical ON TABLE life_event COLUMNS is_canonical;

-- Research Notes
DEFINE TABLE IF NOT EXISTS research_note SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS title       ON TABLE research_note TYPE string;
DEFINE FIELD IF NOT EXISTS description ON TABLE research_note TYPE option<string>;
DEFINE FIELD IF NOT EXISTS body        ON TABLE research_note TYPE option<string>;
DEFINE FIELD IF NOT EXISTS trees       ON TABLE research_note TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS created_by  ON TABLE research_note TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at  ON TABLE research_note TYPE string DEFAULT <string>time::now();
DEFINE FIELD IF NOT EXISTS updated_at  ON TABLE research_note TYPE string DEFAULT <string>time::now();
DEFINE FIELD IF NOT EXISTS is_shared   ON TABLE research_note TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS parent_id   ON TABLE research_note TYPE option<record<research_note>>;
DEFINE INDEX IF NOT EXISTS idx_note_creator ON TABLE research_note COLUMNS created_by;
DEFINE INDEX IF NOT EXISTS idx_note_trees   ON TABLE research_note COLUMNS trees;
DEFINE INDEX IF NOT EXISTS idx_note_shared  ON TABLE research_note COLUMNS is_shared;
DEFINE INDEX IF NOT EXISTS idx_note_parent  ON TABLE research_note COLUMNS parent_id;
-- Backfill is_shared for notes created before the field was added.
UPDATE research_note SET is_shared = false WHERE is_shared IS NONE;

-- Relationship edges: proxy→proxy (Phase 3 switch from person→person).
-- Existing person→person edges remain as inert historical data until Phase 4 cleanup.
DEFINE TABLE OVERWRITE has_father  TYPE RELATION IN person_proxy OUT person_proxy SCHEMAFULL;
DEFINE TABLE OVERWRITE has_mother  TYPE RELATION IN person_proxy OUT person_proxy SCHEMAFULL;
DEFINE TABLE OVERWRITE has_sibling TYPE RELATION IN person_proxy OUT person_proxy SCHEMAFULL;
DEFINE TABLE OVERWRITE has_spouse  TYPE RELATION IN person_proxy OUT person_proxy SCHEMAFULL;
DEFINE FIELD OVERWRITE pedigree ON TABLE has_father  TYPE option<string> ASSERT $value = NONE OR $value IN ["birth", "adopted"];
DEFINE FIELD OVERWRITE pedigree ON TABLE has_mother  TYPE option<string> ASSERT $value = NONE OR $value IN ["birth", "adopted"];
DEFINE FIELD OVERWRITE pedigree ON TABLE has_sibling TYPE option<string> ASSERT $value = NONE OR $value IN ["birth", "half", "adopted"];
DEFINE FIELD IF NOT EXISTS sibling_type  ON TABLE has_sibling TYPE string ASSERT $value IN ["Brother", "Sister"];
DEFINE FIELD IF NOT EXISTS via_parent_id ON TABLE has_sibling TYPE option<string>;
DEFINE FIELD IF NOT EXISTS spouse_from  ON TABLE has_spouse  TYPE option<string>;
DEFINE FIELD IF NOT EXISTS spouse_to    ON TABLE has_spouse  TYPE option<string>;

DEFINE INDEX IF NOT EXISTS unique_father  ON TABLE has_father  COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_mother  ON TABLE has_mother  COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_sibling ON TABLE has_sibling COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_spouse  ON TABLE has_spouse  COLUMNS in, out UNIQUE;

-- Merge proposals: bilateral cross-tree canonical identity merge.
DEFINE TABLE IF NOT EXISTS person_merge_proposal SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS from_proxy_id  ON TABLE person_merge_proposal TYPE record<person_proxy>;
DEFINE FIELD IF NOT EXISTS to_proxy_id    ON TABLE person_merge_proposal TYPE record<person_proxy>;
DEFINE FIELD IF NOT EXISTS proposed_by    ON TABLE person_merge_proposal TYPE string;
DEFINE FIELD IF NOT EXISTS proposed_at    ON TABLE person_merge_proposal TYPE string DEFAULT <string>time::now();
DEFINE FIELD IF NOT EXISTS status         ON TABLE person_merge_proposal TYPE string
  DEFAULT "pending" ASSERT $value IN ["pending", "accepted", "rejected"];
DEFINE FIELD IF NOT EXISTS resolution     ON TABLE person_merge_proposal TYPE option<object>;
DEFINE FIELD IF NOT EXISTS responded_by   ON TABLE person_merge_proposal TYPE option<string>;
DEFINE FIELD IF NOT EXISTS responded_at   ON TABLE person_merge_proposal TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_proposal_status   ON TABLE person_merge_proposal COLUMNS status;
DEFINE INDEX IF NOT EXISTS idx_proposal_from     ON TABLE person_merge_proposal COLUMNS from_proxy_id;
DEFINE INDEX IF NOT EXISTS idx_proposal_to       ON TABLE person_merge_proposal COLUMNS to_proxy_id;
DEFINE INDEX IF NOT EXISTS idx_proposal_proposer ON TABLE person_merge_proposal COLUMNS proposed_by;

-- AI Settings (encrypted BYOK keys; webapp encrypts, server stores opaque blobs)
DEFINE TABLE IF NOT EXISTS user_ai_settings SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS username      ON TABLE user_ai_settings TYPE string;
DEFINE FIELD IF NOT EXISTS provider      ON TABLE user_ai_settings TYPE string;
DEFINE FIELD IF NOT EXISTS model         ON TABLE user_ai_settings TYPE string;
DEFINE FIELD IF NOT EXISTS ollama_url    ON TABLE user_ai_settings TYPE option<string>;
DEFINE FIELD IF NOT EXISTS encrypted_key ON TABLE user_ai_settings TYPE option<string>;
DEFINE FIELD IF NOT EXISTS iv            ON TABLE user_ai_settings TYPE option<string>;
DEFINE FIELD IF NOT EXISTS auth_tag      ON TABLE user_ai_settings TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_ai_settings_username ON TABLE user_ai_settings COLUMNS username UNIQUE;

-- Research Folders (user-scoped; notes store the raw folder ID string as folder_id)
DEFINE TABLE IF NOT EXISTS research_folder SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name       ON TABLE research_folder TYPE string;
DEFINE FIELD IF NOT EXISTS created_by ON TABLE research_folder TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE research_folder TYPE string DEFAULT <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_folder_creator ON TABLE research_folder COLUMNS created_by;
DEFINE INDEX IF NOT EXISTS unique_folder_name ON TABLE research_folder COLUMNS created_by, name UNIQUE;

DEFINE FIELD IF NOT EXISTS folder_id ON TABLE research_note TYPE option<string>;

-- Chat Sessions (AI research assistant conversation history)
DEFINE TABLE IF NOT EXISTS chat_session SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS title      ON TABLE chat_session TYPE string;
DEFINE FIELD IF NOT EXISTS created_by ON TABLE chat_session TYPE string;
DEFINE FIELD IF NOT EXISTS tree       ON TABLE chat_session TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE chat_session TYPE string DEFAULT <string>time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE chat_session TYPE string DEFAULT <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_session_creator ON TABLE chat_session COLUMNS created_by;
DEFINE INDEX IF NOT EXISTS idx_session_tree    ON TABLE chat_session COLUMNS tree;

-- Tree editors: per-tree, per-member write access grants for shared team trees
DEFINE TABLE IF NOT EXISTS tree_editor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS tree_name          ON TABLE tree_editor TYPE string;
DEFINE FIELD IF NOT EXISTS user_id            ON TABLE tree_editor TYPE string;
DEFINE FIELD IF NOT EXISTS granted_by_user_id ON TABLE tree_editor TYPE string;
DEFINE FIELD IF NOT EXISTS granted_at         ON TABLE tree_editor TYPE string DEFAULT <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_tree_editor_tree ON TABLE tree_editor COLUMNS tree_name;
DEFINE INDEX IF NOT EXISTS idx_tree_editor_user ON TABLE tree_editor COLUMNS user_id;
DEFINE INDEX IF NOT EXISTS unique_tree_editor   ON TABLE tree_editor COLUMNS tree_name, user_id UNIQUE;

DEFINE TABLE IF NOT EXISTS chat_message SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS session_id ON TABLE chat_message TYPE record<chat_session>;
DEFINE FIELD IF NOT EXISTS role       ON TABLE chat_message TYPE string ASSERT $value IN ["user", "assistant"];
DEFINE FIELD IF NOT EXISTS content    ON TABLE chat_message TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE chat_message TYPE string DEFAULT <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_message_session ON TABLE chat_message COLUMNS session_id;

-- ---------------------------------------------------------------------------
-- Merge contact requests: cross-user communication before a merge is attempted
-- ---------------------------------------------------------------------------
DEFINE TABLE IF NOT EXISTS merge_contact_request SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS from_proxy_id   ON TABLE merge_contact_request TYPE record<person_proxy>;
DEFINE FIELD IF NOT EXISTS from_user       ON TABLE merge_contact_request TYPE string;
DEFINE FIELD IF NOT EXISTS to_user         ON TABLE merge_contact_request TYPE string;
DEFINE FIELD IF NOT EXISTS initial_message ON TABLE merge_contact_request TYPE option<string>;
DEFINE FIELD IF NOT EXISTS status          ON TABLE merge_contact_request TYPE string DEFAULT 'pending' ASSERT $value IN ['pending', 'accepted', 'ignored'];
DEFINE FIELD IF NOT EXISTS messages        ON TABLE merge_contact_request TYPE option<array<any>>;
DEFINE FIELD IF NOT EXISTS created_at      ON TABLE merge_contact_request TYPE string VALUE <string>time::now() READONLY;
DEFINE FIELD IF NOT EXISTS updated_at      ON TABLE merge_contact_request TYPE string VALUE <string>time::now();
DEFINE INDEX IF NOT EXISTS mcr_to_user   ON TABLE merge_contact_request FIELDS to_user;
DEFINE INDEX IF NOT EXISTS mcr_from_user ON TABLE merge_contact_request FIELDS from_user;

-- ---------------------------------------------------------------------------
-- tack-server notes migration sidecar tables
-- (/Users/colin/.claude/plans/linked-roaming-rabbit.md)
--
-- Folded in from 002_tack_migration_sidecar.surql/003_tack_reverse_drain_
-- state.surql -- both were checked in as standalone numbered migration
-- files, but db.rs's connect() only ever include_str!()s this single
-- schema.surql on startup (confirmed directly: there is no migration-runner
-- anywhere in this crate that scans migrations/ for numbered files, unlike
-- tack-server's own include_dir!-based one). The two files sat unapplied --
-- never created in local dev, staging, or production -- since the day they
-- were added; caught before the backfill/reverse-drain tools (which write
-- to tack_note_meta directly) could fail against a real target for this
-- reason. Matches this file's own established convention (see the
-- "consolidate version and commit missing migration" history) of folding
-- numbered migration files into this one schema and deleting the
-- standalone copies once applied here.
--
-- clann-server keeps its own `/api/notes/*` surface backed by tack-server
-- (see src/tack_client.rs) rather than a clean pass-through, because tack's
-- `Note` schema has no column for `research_note.description`, and because
-- Clann's real per-tree visibility can't be evaluated by tack's ACL alone.
-- These tables carry what tack itself can't:
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
--
--   - tack_reverse_drain_state: reverse-drain's own idempotency marker, a
--     SEPARATE table from tack_migration_state rather than a reused kind
--     value on it -- tack_migration_state's unique index is
--     (surreal_id, kind), keyed by the SurrealDB source row; reverse-drain
--     runs in the opposite direction and needs the opposite key --
--     (tack_id, kind), since a note/folder created fresh in tack after
--     cutover has no research_note/research_folder row to key against
--     until reverse-drain creates one. Keeping them separate also means a
--     rollback rehearsal can never accidentally collide with, or be
--     mistaken for, the forward backfill's own bookkeeping.
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

DEFINE TABLE IF NOT EXISTS tack_reverse_drain_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS tack_id     ON TABLE tack_reverse_drain_state TYPE string;
DEFINE FIELD IF NOT EXISTS kind        ON TABLE tack_reverse_drain_state TYPE string ASSERT $value IN ['folder', 'note'];
DEFINE FIELD IF NOT EXISTS surreal_id  ON TABLE tack_reverse_drain_state TYPE string;
DEFINE FIELD IF NOT EXISTS drained_at  ON TABLE tack_reverse_drain_state TYPE string VALUE <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_reverse_drain_tack ON TABLE tack_reverse_drain_state COLUMNS tack_id, kind UNIQUE;
