DEFINE TABLE IF NOT EXISTS family_tree SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name         ON TABLE family_tree TYPE string;
DEFINE FIELD IF NOT EXISTS display_name ON TABLE family_tree TYPE string;
DEFINE FIELD IF NOT EXISTS owner        ON TABLE family_tree TYPE string;
DEFINE FIELD IF NOT EXISTS is_primary   ON TABLE family_tree TYPE bool DEFAULT false;
DEFINE INDEX IF NOT EXISTS idx_tree_name  ON TABLE family_tree COLUMNS name UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_tree_owner ON TABLE family_tree COLUMNS owner;

DEFINE TABLE IF NOT EXISTS person SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS family_name   ON TABLE person TYPE string;
DEFINE FIELD IF NOT EXISTS first_name    ON TABLE person TYPE string;
DEFINE FIELD IF NOT EXISTS middle_name   ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS sex           ON TABLE person TYPE string ASSERT $value IN ["Male", "Female"];
DEFINE FIELD IF NOT EXISTS date_of_birth  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS place_of_birth ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS date_of_death  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS place_of_death ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS image_path       ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS life_image_path  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS image_bytes      ON TABLE person TYPE option<int>;
DEFINE FIELD IF NOT EXISTS life_image_bytes ON TABLE person TYPE option<int>;
DEFINE FIELD IF NOT EXISTS nickname      ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS username      ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS email         ON TABLE person TYPE option<string>;
DEFINE FIELD verified                    ON TABLE person TYPE bool DEFAULT false VALUE $value OR false;
DEFINE FIELD biography                   ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_by    ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS tree          ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS trees         ON TABLE person TYPE array<string> DEFAULT [];
DEFINE INDEX IF NOT EXISTS idx_created_by  ON TABLE person COLUMNS created_by;
REMOVE INDEX IF EXISTS idx_person_tree ON TABLE person;
DEFINE INDEX IF NOT EXISTS idx_person_trees ON TABLE person COLUMNS trees;

-- Life Events
DEFINE TABLE IF NOT EXISTS life_event SCHEMAFULL;
DEFINE FIELD OVERWRITE person_id ON TABLE life_event TYPE record<person>;
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
DEFINE INDEX IF NOT EXISTS idx_life_event_person ON TABLE life_event COLUMNS person_id;
DEFINE INDEX IF NOT EXISTS idx_life_event_creator ON TABLE life_event COLUMNS created_by;

-- Research Notes
DEFINE TABLE IF NOT EXISTS research_note SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS title       ON TABLE research_note TYPE string;
DEFINE FIELD IF NOT EXISTS description ON TABLE research_note TYPE option<string>;
DEFINE FIELD IF NOT EXISTS body        ON TABLE research_note TYPE option<string>;
DEFINE FIELD IF NOT EXISTS trees       ON TABLE research_note TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS created_by  ON TABLE research_note TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at  ON TABLE research_note TYPE string DEFAULT <string>time::now();
DEFINE FIELD IF NOT EXISTS updated_at  ON TABLE research_note TYPE string DEFAULT <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_note_creator ON TABLE research_note COLUMNS created_by;
DEFINE INDEX IF NOT EXISTS idx_note_trees   ON TABLE research_note COLUMNS trees;

DEFINE TABLE IF NOT EXISTS has_father  TYPE RELATION IN person OUT person SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS has_mother  TYPE RELATION IN person OUT person SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS has_sibling TYPE RELATION IN person OUT person SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS has_spouse  TYPE RELATION IN person OUT person SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS sibling_type ON TABLE has_sibling TYPE string ASSERT $value IN ["Brother", "Sister"];
DEFINE FIELD IF NOT EXISTS spouse_from  ON TABLE has_spouse  TYPE option<string>;
DEFINE FIELD IF NOT EXISTS spouse_to    ON TABLE has_spouse  TYPE option<string>;

DEFINE INDEX IF NOT EXISTS unique_father  ON TABLE has_father  COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_mother  ON TABLE has_mother  COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_sibling ON TABLE has_sibling COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_spouse  ON TABLE has_spouse  COLUMNS in, out UNIQUE;

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

DEFINE TABLE IF NOT EXISTS chat_message SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS session_id ON TABLE chat_message TYPE record<chat_session>;
DEFINE FIELD IF NOT EXISTS role       ON TABLE chat_message TYPE string ASSERT $value IN ["user", "assistant"];
DEFINE FIELD IF NOT EXISTS content    ON TABLE chat_message TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE chat_message TYPE string DEFAULT <string>time::now();
DEFINE INDEX IF NOT EXISTS idx_message_session ON TABLE chat_message COLUMNS session_id;
