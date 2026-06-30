use super::*; // Granular imports
use crate::cli::os::AddOsArgs;
use crate::schema::operating_system::dsl::*;
use diesel::PgConnection;

pub fn add(conn: &mut PgConnection, args: &AddOsArgs) {
    println!("In add action!");
}
