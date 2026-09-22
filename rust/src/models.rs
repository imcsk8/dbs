// Models for DBS supply chain database

#![allow(unused)]
#![allow(clippy::all)]

use crate::schema::*;
use crate::types::*;
use chrono::{DateTime, Utc};
use diesel::deserialize::FromSql;
use diesel::prelude::*;
use diesel::serialize::ToSql;
use diesel::sql_types::Jsonb;
use diesel::AsExpression;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = architecture)]
pub struct Architecture {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = operating_system)]
pub struct OperatingSystem {
    pub id: i32,
    pub name: String,
    pub version: String,
    pub system_type: OsType,
    pub release: String,
    pub architecture: i32,
    pub summary: String,
    pub url: String,
    pub license: String,
    pub description: String,
    pub manager: i32,
    pub distro_tag: Option<String>,
    pub dist_git_url_template: Option<String>,
    pub dist_git_branch: Option<String>,
    pub lookaside_cache_url: Option<String>,
    pub api_type: Option<String>,
    pub api_url: Option<String>,
    pub mock_chroot: Option<String>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = operating_system_package)]
pub struct OperatingSystemPackage {
    pub id: i32,
    pub id_os: Option<i32>,
    pub id_package: Option<i32>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = package)]
pub struct Package {
    pub id: i32,
    pub name: String,
    pub epoch: i32,
    pub version: String,
    pub release: String,
    pub architecture: i32,
    pub package_size: String,
    pub file_size_bytes: i64,
    pub source: String,
    pub repository: String,
    pub summary: String,
    pub url: String,
    pub license: String,
    pub description: String,
    pub in_repo: Option<bool>,
    pub created: Option<bool>,
    pub vulnerable: Option<bool>,
    pub build_status: BuildStatus,
    pub build_duration_seconds: Option<f32>,
    pub build_log_path: Option<String>,
    pub error_summary: Option<String>,
    pub worker_id: Option<i32>,
    pub sourcerpm: Option<String>,
    pub dist_git_url: Option<String>,
    pub dist_git_branch: Option<String>,
    pub dist_git_commit: Option<String>,
    pub spec_file: Option<String>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = package_dependencies)]
pub struct PackageDependency {
    pub id: i32,
    pub id_package: Option<i32>,
    pub id_dependency: Option<i32>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = package_provides)]
pub struct PackageProvides {
    pub id: i32,
    pub id_package: i32,
    pub name: String,
    pub flags: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = package_requires)]
pub struct PackageRequires {
    pub id: i32,
    pub id_package: i32,
    pub name: String,
    pub flags: Option<String>,
    pub version: Option<String>,
    pub is_build_require: bool,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = package_artifact)]
pub struct PackageArtifact {
    pub id: i32,
    pub id_package: Option<i32>,
    pub rpm_filename: String,
    pub rpm_path: String,
    pub arch: String,
    pub is_source: bool,
    pub file_size_bytes: i64,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = package_manager)]
pub struct PackageManager {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable, Insertable, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = product)]
pub struct Product {
    pub id: i32,
    pub id_os: i32,
    pub format: OsFormat,
}

// ---------------------------------------------------------------------------
// Insertable models for rows without serial ID
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Insertable, Serialize, Deserialize)]
#[diesel(table_name = operating_system)]
pub struct NewOperatingSystem {
    pub name: String,
    pub version: String,
    pub system_type: OsType,
    pub release: String,
    pub architecture: i32,
    pub summary: String,
    pub url: String,
    pub license: String,
    pub description: String,
    pub manager: i32,
    pub distro_tag: Option<String>,
    pub dist_git_url_template: Option<String>,
    pub dist_git_branch: Option<String>,
    pub lookaside_cache_url: Option<String>,
    pub api_type: Option<String>,
    pub api_url: Option<String>,
    pub mock_chroot: Option<String>,
}

#[derive(Clone, Debug, Insertable, Serialize, Deserialize)]
#[diesel(table_name = package)]
pub struct NewPackage {
    pub name: String,
    pub epoch: i32,
    pub version: String,
    pub release: String,
    pub architecture: i32,
    pub package_size: String,
    pub file_size_bytes: i64,
    pub source: String,
    pub repository: String,
    pub summary: String,
    pub url: String,
    pub license: String,
    pub description: String,
    pub in_repo: Option<bool>,
    pub created: Option<bool>,
    pub vulnerable: Option<bool>,
    pub build_status: BuildStatus,
    pub build_duration_seconds: Option<f32>,
    pub build_log_path: Option<String>,
    pub error_summary: Option<String>,
    pub worker_id: Option<i32>,
    pub sourcerpm: Option<String>,
    pub dist_git_url: Option<String>,
    pub dist_git_branch: Option<String>,
    pub dist_git_commit: Option<String>,
    pub spec_file: Option<String>,
}

#[derive(Clone, Debug, Insertable, Serialize, Deserialize)]
#[diesel(table_name = package_provides)]
pub struct NewPackageProvides {
    pub id_package: i32,
    pub name: String,
    pub flags: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Insertable, Serialize, Deserialize)]
#[diesel(table_name = package_requires)]
pub struct NewPackageRequires {
    pub id_package: i32,
    pub name: String,
    pub flags: Option<String>,
    pub version: Option<String>,
    pub is_build_require: bool,
}

#[derive(Clone, Debug, Insertable, Serialize, Deserialize)]
#[diesel(table_name = package_artifact)]
pub struct NewPackageArtifact {
    pub id_package: Option<i32>,
    pub rpm_filename: String,
    pub rpm_path: String,
    pub arch: String,
    pub is_source: bool,
    pub file_size_bytes: i64,
}

