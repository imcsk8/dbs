// In your schema.rs or a dedicated types.rs file
use diesel::prelude::*;
use diesel::{AsExpression,SqlType,deserialize,serialize,FromSqlRow};
use diesel::deserialize::FromSql;
use diesel::pg::{Pg,PgValue};
use diesel::serialize::{IsNull,Output,ToSql};
use std::io::Write;
use serde::{Serialize, Deserialize};
use std::fmt::{self, Debug, Display};
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

/// Convert to Display (and automatically ToString)
impl Display for OsType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            OsType::VERSIONED => write!(f, "VERSIONED"),
            OsType::ROLLING => write!(f, "ROLLING"),
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

/// Convert to Display (and automatically ToString)
impl Display for OsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            OsFormat::ISO  => write!(f, "ISO"),
            OsFormat::OCI  => write!(f, "OCI"),
            OsFormat::QCOW => write!(f, "QCOW"),
            OsFormat::RAW  => write!(f, "RAW"),
        }
    }
}

#[derive(Debug, Clone, Copy, SqlType, QueryId)]
#[diesel(postgres_type(name = "build_status", schema = "public"))]
#[diesel(sql_type = BuildStatus)]
pub struct BuildStatusType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsExpression, FromSqlRow, Serialize, Deserialize, ValueEnum)]
#[diesel(sql_type = BuildStatusType)]
pub enum BuildStatus {
    PENDING,
    BUILDING,
    SUCCESS,
    FAILED,
    SKIPPED,
}

impl ToSql<BuildStatusType, Pg> for BuildStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        match *self {
            BuildStatus::PENDING  => out.write_all(b"PENDING")?,
            BuildStatus::BUILDING => out.write_all(b"BUILDING")?,
            BuildStatus::SUCCESS  => out.write_all(b"SUCCESS")?,
            BuildStatus::FAILED   => out.write_all(b"FAILED")?,
            BuildStatus::SKIPPED  => out.write_all(b"SKIPPED")?,
        }
        Ok(IsNull::No)
    }
}

impl FromSql<BuildStatusType, Pg> for BuildStatus {
    fn from_sql(value: PgValue<'_>) -> deserialize::Result<Self> {
        match value.as_bytes() {
            b"PENDING"  => Ok(BuildStatus::PENDING),
            b"BUILDING" => Ok(BuildStatus::BUILDING),
            b"SUCCESS"  => Ok(BuildStatus::SUCCESS),
            b"FAILED"   => Ok(BuildStatus::FAILED),
            b"SKIPPED"  => Ok(BuildStatus::SKIPPED),
            _ => Err("Unrecognized build_status enum variant".into()),
        }
    }
}

impl From<String> for BuildStatus {
    fn from(item: String) -> Self {
        Self::from(item.as_str())
    }
}

impl From<&str> for BuildStatus {
    fn from(item: &str) -> Self {
        match item.to_uppercase().as_str() {
            "PENDING"  => BuildStatus::PENDING,
            "BUILDING" => BuildStatus::BUILDING,
            "SUCCESS"  => BuildStatus::SUCCESS,
            "FAILED"   => BuildStatus::FAILED,
            "SKIPPED"  => BuildStatus::SKIPPED,
            _ => BuildStatus::PENDING,
        }
    }
}

impl Display for BuildStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            BuildStatus::PENDING  => write!(f, "PENDING"),
            BuildStatus::BUILDING => write!(f, "BUILDING"),
            BuildStatus::SUCCESS  => write!(f, "SUCCESS"),
            BuildStatus::FAILED   => write!(f, "FAILED"),
            BuildStatus::SKIPPED  => write!(f, "SKIPPED"),
        }
    }
}
