use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::sql_query;
use crate::models::Architecture;
//use cli::{Cli, Commands, OsCommands, PkgCommands};
use crate::cli::{Cli, Commands, os::OsCommands, os_actions};
use clap::Parser;

use std::env;

pub mod models;
pub mod schema;
pub mod types;
pub mod cli;

#[tokio::main]
async fn main() -> eyre::Result<()> {

    let cli = Cli::parse();
    //let mut conn = establish_connection();
    
    println!("---------  COMMAND: {:?}", cli.command);

    match &cli.command {
        Commands::Os(os_commands) => match &os_commands.command {
            // Os Commands
            OsCommands::Add(args) => {
                //os_actions::add(&mut conn, args);
                println!("Os Command not implemented");

            },
            _ => todo!("Os Command not implemented"),
        },
        _ => todo!("Command not implemented"),

    }

    let target_arch = "x86_64";

    println!("\n🔍 Searching for architecture: {}", target_arch);

    // Define the raw SQL query with a bind parameter ($1)
    let query = "SELECT id, name, description FROM architecture WHERE name = $1";
/*
    // Execute the query
    let results = sql_query(query)
        .bind::<diesel::sql_types::Text, _>(target_arch)
        .load::<Architecture>(&mut conn)
        .expect("Error executing raw query");

    // Print the results
    for arch in results {
        println!("{:?}", arch);
    }

*/
    Ok(())
}

pub fn establish_connection() -> PgConnection {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    PgConnection::establish(&database_url)
        .unwrap_or_else(|_| panic!("Error connecting to {}", database_url))
}
