//! Database schema bootstrapping, status inspection, and migration management.
//!
//! Embeds the supply-chain database schema directly into the binary at compile time,
//! enabling zero-dependency database provisioning without requiring external SQL files,
//! `psql`, or Makefiles.

use diesel::connection::SimpleConnection;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Text};
use eyre::{eyre, Result};

/// Embedded schema creation and seed data SQL script from migrations.
pub const SCHEMA_UP: &str = include_str!("../../migrations/2025-07-03-050157_supply_chain/up.sql");

/// Embedded schema teardown SQL script from migrations.
pub const SCHEMA_DOWN: &str = include_str!("../../migrations/2025-07-03-050157_supply_chain/down.sql");

#[derive(QueryableByName, Debug)]
struct ExistsResult {
    #[diesel(sql_type = Bool)]
    exists: bool,
}

#[derive(QueryableByName, Debug)]
struct VersionResult {
    #[diesel(sql_type = Text)]
    version: String,
}

#[derive(QueryableByName, Debug)]
struct CountResult {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName, Debug)]
struct DbNameResult {
    #[diesel(sql_type = Text)]
    db_name: String,
}

/// Status report containing metadata and table row counts for a DBS database.
#[derive(Debug, Clone)]
pub struct DbStatusReport {
    pub database_name: String,
    pub server_version: String,
    pub is_initialized: bool,
    pub package_count: i64,
    pub os_count: i64,
    pub arch_count: i64,
    pub pm_count: i64,
    pub artifact_count: i64,
}

/// Checks whether the core schema (`package` table) has already been created.
pub fn is_schema_initialized(conn: &mut PgConnection) -> Result<bool> {
    let result = diesel::sql_query(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.tables 
            WHERE table_schema = 'public' AND table_name = 'package'
        ) AS exists;",
    )
    .get_result::<ExistsResult>(conn)?;

    Ok(result.exists)
}

/// Initializes the database schema and seeds default data.
///
/// If `force` is false and the schema already exists, it aborts without overwriting
/// to prevent accidental data loss.
pub fn bootstrap_database(conn: &mut PgConnection, force: bool) -> Result<()> {
    let already_initialized = is_schema_initialized(conn).unwrap_or(false);

    if already_initialized {
        if !force {
            return Err(eyre!(
                "Database schema is already initialized with existing tables. \
                 Use 'dbs db reset --force' to recreate or pass '--force' to overwrite."
            ));
        }
        println!("⚠️  --force specified: dropping existing schema before re-initialization...");
        conn.batch_execute(SCHEMA_DOWN)
            .map_err(|e| eyre!("Failed to drop existing schema: {}", e))?;
    }

    println!("Applying embedded DBS schema from rust/migrations/2025-07-03-050157_supply_chain/up.sql...");
    conn.batch_execute(SCHEMA_UP)
        .map_err(|e| eyre!("Failed to execute embedded schema_up SQL: {}", e))?;

    println!("✅ Database schema initialized and seeded successfully.");
    Ok(())
}

/// Drops all existing tables, enums, and dependencies, then reapplies the clean schema.
pub fn reset_database(conn: &mut PgConnection) -> Result<()> {
    println!("Dropping existing DBS database schema...");
    conn.batch_execute(SCHEMA_DOWN)
        .map_err(|e| eyre!("Failed to execute schema_down SQL: {}", e))?;

    println!("Re-applying clean DBS database schema and seeds...");
    conn.batch_execute(SCHEMA_UP)
        .map_err(|e| eyre!("Failed to execute schema_up SQL: {}", e))?;

    println!("✅ Database reset completed successfully.");
    Ok(())
}

/// Gathers connectivity and table statistics for the connected database.
pub fn get_database_status(conn: &mut PgConnection) -> Result<DbStatusReport> {
    let db_name = diesel::sql_query("SELECT current_database() AS db_name;")
        .get_result::<DbNameResult>(conn)
        .map(|r| r.db_name)
        .unwrap_or_else(|_| "unknown".to_string());

    let version = diesel::sql_query("SELECT version() AS version;")
        .get_result::<VersionResult>(conn)
        .map(|r| r.version)
        .unwrap_or_else(|_| "unknown".to_string());

    let initialized = is_schema_initialized(conn).unwrap_or(false);

    let (package_count, os_count, arch_count, pm_count, artifact_count) = if initialized {
        let pkg = diesel::sql_query("SELECT COUNT(*) AS count FROM package;")
            .get_result::<CountResult>(conn)
            .map(|r| r.count)
            .unwrap_or(0);
        let os = diesel::sql_query("SELECT COUNT(*) AS count FROM operating_system;")
            .get_result::<CountResult>(conn)
            .map(|r| r.count)
            .unwrap_or(0);
        let arch = diesel::sql_query("SELECT COUNT(*) AS count FROM architecture;")
            .get_result::<CountResult>(conn)
            .map(|r| r.count)
            .unwrap_or(0);
        let pm = diesel::sql_query("SELECT COUNT(*) AS count FROM package_manager;")
            .get_result::<CountResult>(conn)
            .map(|r| r.count)
            .unwrap_or(0);
        let art = diesel::sql_query("SELECT COUNT(*) AS count FROM package_artifact;")
            .get_result::<CountResult>(conn)
            .map(|r| r.count)
            .unwrap_or(0);
        (pkg, os, arch, pm, art)
    } else {
        (0, 0, 0, 0, 0)
    };

    Ok(DbStatusReport {
        database_name: db_name,
        server_version: version,
        is_initialized: initialized,
        package_count,
        os_count,
        arch_count,
        pm_count,
        artifact_count,
    })
}

/// Returns the embedded raw SQL string for manual DBA inspection or external piping.
pub fn dump_schema(down: bool) -> &'static str {
    if down {
        SCHEMA_DOWN
    } else {
        SCHEMA_UP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_schema_scripts_not_empty() {
        assert!(!SCHEMA_UP.is_empty(), "Embedded up.sql must not be empty");
        assert!(!SCHEMA_DOWN.is_empty(), "Embedded down.sql must not be empty");
        assert!(
            SCHEMA_UP.contains("CREATE TABLE IF NOT EXISTS package"),
            "Embedded up.sql must contain package table definition"
        );
        assert!(
            SCHEMA_UP.contains("INSERT INTO package_manager"),
            "Embedded up.sql must contain seed data"
        );
        assert!(
            SCHEMA_DOWN.contains("DROP TABLE IF EXISTS package"),
            "Embedded down.sql must contain DROP statements"
        );
    }

    #[test]
    fn test_dump_schema() {
        assert_eq!(dump_schema(false), SCHEMA_UP);
        assert_eq!(dump_schema(true), SCHEMA_DOWN);
    }
}
