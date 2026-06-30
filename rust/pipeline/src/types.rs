// In your schema.rs or a dedicated types.rs file
use diesel::prelude::*;
use diesel::{AsExpression,SqlType,deserialize,serialize,FromSqlRow};
use diesel::deserialize::FromSql;
use diesel::pg::{Pg,PgValue};
use diesel::serialize::{IsNull,Output,ToSql};
use std::io::Write;
use serde::{Serialize, Deserialize};
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use diesel::QueryId;
use clap::ValueEnum;


// Enum for types of reactions
#[derive(Debug, Clone, Copy, SqlType, QueryId)]
#[diesel(postgres_type(name = "os_type", schema = "public"))]
#[diesel(sql_type = OsType)]
pub struct OsTypeType;

// Step 2: Create the Rust enum
//#[derive(Clone, Debug, AsExpression, Serialize, Deserialize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, AsExpression, FromSqlRow, Serialize, Deserialize, ValueEnum)]
#[diesel(sql_type = OsTypeType)]
pub enum OsType {
    VERSIONED,
    ROLLING,
}

// Step 3: Implementation for serialization (Rust -> DB)
impl ToSql<OsTypeType, Pg> for OsType {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        match *self {
            OsType::VERSIONED => out.write_all(b"VERSIONED")?,
            OsType::ROLLING   => out.write_all(b"ROLLING")?,
        }
        Ok(IsNull::No)
    }
}

// Step 4: Implementation for deserialization (DB -> Rust)
impl FromSql<OsTypeType, Pg> for OsType {
    fn from_sql(value: PgValue<'_>) -> deserialize::Result<Self> {
        match value.as_bytes() {
            b"VERSIONED" => Ok(OsType::VERSIONED),
            b"ROLLING" => Ok(OsType::ROLLING),
            _ => Err("Unrecognized enum variant".into()),
        }
    }
}

 /// For converting from String
impl From<String> for OsType {
    fn from(item: String) -> Self {
        Self::from(item.as_str())
    }
}

/// Convert from &str
impl From<&str> for OsType {
    fn from(item: &str) -> Self {
        match item {
            "VERSIONED" => OsType::VERSIONED,
            "ROLLING" => OsType::ROLLING,
            &_ => panic!("Wrong OsType options: VERSIONED, ROLLING"),
        }
    }
}

/// Convert to String
impl ToString for OsType {
    fn to_string(&self) -> String {
        match *self {
            OsType::VERSIONED => String::from("VERSIONED"),
            OsType::ROLLING => String::from("ROLLING"),
        }
    }
}


#[derive(Debug, Clone, Copy, SqlType)]
#[diesel(check_for_backend(Pg))]
#[diesel(postgres_type(name = "os_format"))]
#[diesel(sql_type = OsFormat)]
pub struct OsFormatType;

impl Expression for OsFormatType {
    type SqlType = OsFormatType;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsExpression, FromSqlRow, Serialize, Deserialize)]
#[diesel(sql_type = OsFormatType)]
pub enum OsFormat {
    ISO,
    OCI,
    QCOW,
    RAW,
}

// Step 3: Implementation for serialization (Rust -> DB)
impl ToSql<OsFormatType, Pg> for OsFormat {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        match *self {
            OsFormat::ISO  => out.write_all(b"ISO")?,
            OsFormat::OCI  => out.write_all(b"OCI")?,
            OsFormat::QCOW => out.write_all(b"QCOW")?,
            OsFormat::RAW  => out.write_all(b"RAW")?,
        }
        Ok(IsNull::No)
    }
}

impl FromSql<OsFormatType, Pg> for OsFormat {
    fn from_sql(value: PgValue<'_>) -> deserialize::Result<Self> {
        match value.as_bytes() {
            b"ISO"  => Ok(OsFormat::ISO),
            b"OCI"  => Ok(OsFormat::OCI),
            b"QCOW" => Ok(OsFormat::QCOW),
            b"RAW"  => Ok(OsFormat::RAW),
            _ => Err("Unrecognized enum variant".into()),
        }
    }
}


 /// For converting from String
impl From<String> for OsFormat {
    fn from(item: String) -> Self {
        Self::from(item.as_str())
    }
}

/// Convert from &str
impl From<&str> for OsFormat {
    fn from(item: &str) -> Self {
        match item {
            "ISO"  => OsFormat::ISO,
            "OCI"  => OsFormat::OCI,
            "QCOW" => OsFormat::QCOW,
            "RAW"  => OsFormat::RAW,
            &_ => panic!("Wrong OsFormat options are: ISO, QCOW, RAW, OCI"),
        }
    }
}

/// Convert to String
impl ToString for OsFormat {
    fn to_string(&self) -> String {
        match *self {
            OsFormat::ISO  => String::from("ISO"),
            OsFormat::OCI  => String::from("OCI"),
            OsFormat::QCOW => String::from("QCOW"),
            OsFormat::RAW  => String::from("RAW"),
        }
    }
}
