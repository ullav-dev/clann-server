DEFINE TABLE IF NOT EXISTS person SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS family_name   ON TABLE person TYPE string;
DEFINE FIELD IF NOT EXISTS first_name    ON TABLE person TYPE string;
DEFINE FIELD IF NOT EXISTS middle_name   ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS sex           ON TABLE person TYPE string ASSERT $value IN ["Male", "Female"];
DEFINE FIELD IF NOT EXISTS date_of_birth  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS place_of_birth ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS date_of_death  ON TABLE person TYPE option<string>;
DEFINE FIELD IF NOT EXISTS place_of_death ON TABLE person TYPE option<string>;

DEFINE TABLE IF NOT EXISTS has_father  TYPE RELATION IN person OUT person SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS has_mother  TYPE RELATION IN person OUT person SCHEMAFULL;
DEFINE TABLE IF NOT EXISTS has_sibling TYPE RELATION IN person OUT person SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS sibling_type ON TABLE has_sibling TYPE string ASSERT $value IN ["Brother", "Sister"];

DEFINE INDEX IF NOT EXISTS unique_father  ON TABLE has_father  COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_mother  ON TABLE has_mother  COLUMNS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS unique_sibling ON TABLE has_sibling COLUMNS in, out UNIQUE;
