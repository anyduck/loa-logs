use lazy_static::lazy_static;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

lazy_static! {
    pub static ref MIGRATIONS: Migrations<'static> = Migrations::new(vec![
        M::up("
            CREATE TABLE encounter (
                id INTEGER PRIMARY KEY,
                last_combat_packet INTEGER,
                fight_start INTEGER,
                local_player TEXT,
                current_boss TEXT,
                duration INTEGER,
                total_damage_dealt INTEGER,
                top_damage_dealt INTEGER,
                total_damage_taken INTEGER,
                top_damage_taken INTEGER,
                dps INTEGER,
                buffs TEXT,
                debuffs TEXT
            );

            CREATE TABLE entity (
                name TEXT,
                encounter_id INTEGER NOT NULL,
                npc_id INTEGER,
                entity_type TEXT,
                class_id INTEGER,
                class TEXT,
                gear_score REAL,
                current_hp INTEGER,
                max_hp INTEGER,
                is_dead INTEGER,
                skills TEXT,
                damage_stats TEXT,
                skill_stats TEXT,
                last_update INTEGER,
                PRIMARY KEY (name, encounter_id),
                FOREIGN KEY (encounter_id) REFERENCES encounter (id) ON DELETE CASCADE
            );

            CREATE INDEX encounter_fight_start_index ON encounter (fight_start DESC);
            CREATE INDEX encounter_current_boss_index ON encounter (current_boss);
            CREATE INDEX entity_encounter_id_index ON entity (encounter_id DESC);
            CREATE INDEX entity_name_index ON entity (name);
            CREATE INDEX entity_class_index ON entity (class);
        "),
        M::up("ALTER TABLE encounter ADD COLUMN misc TEXT;"),
        M::up("ALTER TABLE encounter ADD COLUMN difficulty TEXT;"),
        M::up("
            ALTER TABLE encounter ADD COLUMN version INTEGER DEFAULT 1;
            ALTER TABLE encounter ADD COLUMN cleared BOOLEAN;
            ALTER TABLE encounter ADD COLUMN favorite BOOLEAN DEFAULT 0;
            CREATE INDEX encounter_favorite_index ON encounter (favorite);
        "),
        M::up("
            ALTER TABLE entity ADD COLUMN dps INTEGER;
            UPDATE entity SET dps = coalesce(json_extract(damage_stats, '$.dps'), 0) WHERE dps IS NULL;
            UPDATE encounter SET cleared = coalesce(json_extract(misc, '$.raidClear'), 0) WHERE cleared IS NULL;
        "),
        M::up("ALTER TABLE encounter ADD COLUMN boss_only_damage BOOLEAN NOT NULL DEFAULT 0;"),
        M::up("
            ALTER TABLE encounter ADD COLUMN total_shielding INTEGER DEFAULT 0;
            ALTER TABLE encounter ADD COLUMN total_effective_shielding INTEGER DEFAULT 0;
            ALTER TABLE encounter ADD COLUMN applied_shield_buffs TEXT;
        "),
        M::up("ALTER TABLE entity ADD COLUMN character_id INTEGER;"),
        M::up("ALTER TABLE entity ADD COLUMN engravings TEXT;"),
        M::up("ALTER TABLE entity ADD COLUMN gear_hash TEXT;"),
        M::up("
            CREATE TABLE encounter_preview (
                id INTEGER PRIMARY KEY,
                fight_start INTEGER,
                current_boss TEXT,
                duration INTEGER,
                players TEXT,
                difficulty TEXT,
                local_player TEXT,
                my_dps INTEGER,
                favorite BOOLEAN NOT NULL DEFAULT 0,
                cleared BOOLEAN,
                boss_only_damage BOOLEAN NOT NULL DEFAULT 0,
                FOREIGN KEY (id) REFERENCES encounter(id) ON DELETE CASCADE
            );

            INSERT INTO encounter_preview SELECT
                id, fight_start, current_boss, duration, 
                (
                    SELECT GROUP_CONCAT(class_id || ':' || name ORDER BY dps DESC)
                    FROM entity
                    WHERE encounter_id = encounter.id AND entity_type = 'PLAYER'
                ) AS players,
                difficulty, local_player,
                (
                    SELECT dps
                    FROM entity
                    WHERE encounter_id = encounter.id AND name = encounter.local_player
                ) AS my_dps,
                favorite, cleared, boss_only_damage
            FROM encounter;

            DROP INDEX IF EXISTS encounter_fight_start_index;
            DROP INDEX IF EXISTS encounter_current_boss_index;
            DROP INDEX IF EXISTS encounter_favorite_index;
            DROP INDEX IF EXISTS entity_name_index;
            DROP INDEX IF EXISTS entity_class_index;

            ALTER TABLE encounter DROP COLUMN fight_start;
            ALTER TABLE encounter DROP COLUMN current_boss;
            ALTER TABLE encounter DROP COLUMN duration;
            ALTER TABLE encounter DROP COLUMN difficulty;
            ALTER TABLE encounter DROP COLUMN local_player;
            ALTER TABLE encounter DROP COLUMN favorite;
            ALTER TABLE encounter DROP COLUMN cleared;
            ALTER TABLE encounter DROP COLUMN boss_only_damage;

            CREATE INDEX encounter_preview_favorite_index ON encounter_preview(favorite);
            CREATE INDEX encounter_preview_fight_start_index ON encounter_preview(fight_start);
            CREATE INDEX encounter_preview_my_dps_index ON encounter_preview(my_dps);
            CREATE INDEX encounter_preview_duration_index ON encounter_preview(duration);
        ").comment("move encounter preview info into a separate table"),
        M::up("
            CREATE VIRTUAL TABLE encounter_search USING fts5(
                current_boss, players, columnsize=0, detail=full,
                tokenize='trigram remove_diacritics 1',
                content=encounter_preview, content_rowid=id
            );
            INSERT INTO encounter_search(encounter_search) VALUES('rebuild');
            CREATE TRIGGER encounter_preview_ai AFTER INSERT ON encounter_preview BEGIN
                INSERT INTO encounter_search(rowid, current_boss, players)
                VALUES (new.id, new.current_boss, new.players);
            END;
            CREATE TRIGGER encounter_preview_ad AFTER DELETE ON encounter_preview BEGIN
                INSERT INTO encounter_search(encounter_search, rowid, current_boss, players)
                VALUES('delete', old.id, old.current_boss, old.players);
            END;
            CREATE TRIGGER encounter_preview_au AFTER UPDATE OF current_boss, players ON encounter_preview BEGIN
                INSERT INTO encounter_search(encounter_search, rowid, current_boss, players)
                VALUES('delete', old.id, old.current_boss, old.players);
                INSERT INTO encounter_search(rowid, current_boss, players)
                VALUES (new.id, new.current_boss, new.players);
            END;
        ").comment("add full text search"),
        M::up("
            ALTER TABLE encounter ADD COLUMN boss_hp_log BLOB;
            ALTER TABLE encounter ADD COLUMN stagger_log TEXT;
        ").comment("add compression to logs"),
        M::up("
            CREATE TABLE IF NOT EXISTS sync_logs (
                encounter_id INTEGER PRIMARY KEY,
                upstream_id TEXT,
                failed BOOLEAN NOT NULL DEFAULT 0,
                FOREIGN KEY (encounter_id) REFERENCES encounter (id) ON DELETE CASCADE
            );
        ").comment("allow uploading logs"),
        M::up("
            ALTER TABLE entity ADD COLUMN spec TEXT;
            ALTER TABLE entity ADD COLUMN ark_passive_active BOOLEAN;
            ALTER TABLE entity ADD COLUMN ark_passive_data TEXT;
        "),
    ]);
}

/// Maps the state of database schema to [rusqlite_migration] version number
fn get_legacy_version(conn: &Connection) -> Result<usize, rusqlite::Error> {
    let new_table_columns = [
        // https://github.com/snoww/loa-logs/tree/fb1fb86291e22afac74c4f948888f
        ["encounter", "id"],
        // https://github.com/snoww/loa-logs/commit/e9b89b18ddc27eec6f51dcfbf51
        ["encounter", "misc"],
        // https://github.com/snoww/loa-logs/commit/8232b184131c27f8a272534d6a8
        ["encounter", "difficulty"],
        // https://github.com/snoww/loa-logs/commit/e2e948ef9a92e76f3f5801687f8
        // https://github.com/snoww/loa-logs/commit/1fc86b2faafaef5db57758a378c
        ["encounter", "version"],
        // https://github.com/snoww/loa-logs/commit/c483c53b2c078935b984c2a051c
        ["entity", "dps"],
        // https://github.com/snoww/loa-logs/commit/fe8757b6e98186ba261fb692bab
        ["encounter", "boss_only_damage"],
        // https://github.com/snoww/loa-logs/commit/cff6859e41bc7c279a5d75a8a4b
        // https://github.com/snoww/loa-logs/commit/904a43f78b1ce7dffec70e3b22f
        // https://github.com/snoww/loa-logs/commit/beb178a64654e8e32d5c1949cda
        ["encounter", "total_shielding"],
        // https://github.com/snoww/loa-logs/commit/24417ebe6f3f136bba92f2a44e1
        ["entity", "character_id"],
        // https://github.com/snoww/loa-logs/commit/c42a9632350efe49d46f5fd92bb
        ["entity", "engravings"],
        // https://github.com/snoww/loa-logs/commit/c2165dcea486e7b619e66eb32d3
        ["entity", "gear_hash"],
        // https://github.com/snoww/loa-logs/commit/445c2532f86e4aa345c59adbe5a
        ["encounter_preview", "id"],
        // https://github.com/snoww/loa-logs/commit/9e8905578886fe1db1a9d68f606
        ["encounter_search_data", "id"],
        // https://github.com/snoww/loa-logs/commit/b6367a1b004ecf7be292020b2bf
        ["encounter", "boss_hp_log"],
        // https://github.com/snoww/loa-logs/commit/90fa86ff41ca35a8b4507af78ce
        ["sync_logs", "encounter_id"],
        // https://github.com/snoww/loa-logs/commit/ec6b9740a0c4ed2433a2e87d4f5
        // https://github.com/snoww/loa-logs/commit/7b50723659a47e31f95f6ce2971
        ["entity", "spec"],
    ];

    let mut table_column_stmt = conn.prepare("SELECT 1 FROM pragma_table_info(?) WHERE name=?")?;

    for (version, table_column) in new_table_columns.iter().enumerate().rev() {
        if table_column_stmt.exists(table_column)? {
            return Ok(version + 1);
        }
    }

    Ok(0)
}

pub fn ensure_version_is_set(conn: &Connection) -> Result<(), rusqlite::Error> {
    let version: usize = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version == 0 {
        conn.pragma_update(None, "user_version", get_legacy_version(conn)?)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_migrations_test() {
        assert!(MIGRATIONS.validate().is_ok());
    }

    #[test]
    fn get_legacy_version_test() {
        for version in 0..=15 {
            let conn = &mut Connection::open_in_memory().unwrap();
            MIGRATIONS.to_version(conn, version).unwrap();
            assert_eq!(get_legacy_version(conn), Ok(version), "on v={version}");
        }
    }
}
