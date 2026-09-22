// @generated automatically by Diesel CLI and enhanced for DBS.

pub mod sql_types {
    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "os_format"))]
    pub struct OsFormat;

    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "os_type"))]
    pub struct OsType;

    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "build_status"))]
    pub struct BuildStatus;
}

diesel::table! {
    architecture (id) {
        id -> Int4,
        name -> Text,
        description -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::OsType;
    use crate::types::OsTypeType;

    operating_system (id) {
        id -> Int4,
        name -> Text,
        version -> Text,
        system_type -> OsTypeType,
        release -> Text,
        architecture -> Int4,
        summary -> Text,
        url -> Text,
        license -> Text,
        description -> Text,
        manager -> Int4,
        distro_tag -> Nullable<Text>,
        dist_git_url_template -> Nullable<Text>,
        dist_git_branch -> Nullable<Text>,
        lookaside_cache_url -> Nullable<Text>,
        api_type -> Nullable<Text>,
        api_url -> Nullable<Text>,
        mock_chroot -> Nullable<Text>,
    }
}

diesel::table! {
    operating_system_package (id) {
        id -> Int4,
        id_os -> Nullable<Int4>,
        id_package -> Nullable<Int4>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BuildStatus;
    use crate::types::BuildStatusType;

    package (id) {
        id -> Int4,
        name -> Text,
        epoch -> Int4,
        version -> Text,
        release -> Text,
        architecture -> Int4,
        package_size -> Text,
        file_size_bytes -> Int8,
        source -> Text,
        repository -> Text,
        summary -> Text,
        url -> Text,
        license -> Text,
        description -> Text,
        in_repo -> Nullable<Bool>,
        created -> Nullable<Bool>,
        vulnerable -> Nullable<Bool>,
        build_status -> BuildStatusType,
        build_duration_seconds -> Nullable<Float4>,
        build_log_path -> Nullable<Text>,
        error_summary -> Nullable<Text>,
        worker_id -> Nullable<Int4>,
        sourcerpm -> Nullable<Text>,
        dist_git_url -> Nullable<Text>,
        dist_git_branch -> Nullable<Text>,
        dist_git_commit -> Nullable<Text>,
        spec_file -> Nullable<Text>,
    }
}

diesel::table! {
    package_dependencies (id) {
        id -> Int4,
        id_package -> Nullable<Int4>,
        id_dependency -> Nullable<Int4>,
    }
}

diesel::table! {
    package_provides (id) {
        id -> Int4,
        id_package -> Int4,
        name -> Text,
        flags -> Nullable<Text>,
        version -> Nullable<Text>,
    }
}

diesel::table! {
    package_requires (id) {
        id -> Int4,
        id_package -> Int4,
        name -> Text,
        flags -> Nullable<Text>,
        version -> Nullable<Text>,
        is_build_require -> Bool,
    }
}

diesel::table! {
    package_artifact (id) {
        id -> Int4,
        id_package -> Nullable<Int4>,
        rpm_filename -> Text,
        rpm_path -> Text,
        arch -> Text,
        is_source -> Bool,
        file_size_bytes -> Int8,
        created_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    package_manager (id) {
        id -> Int4,
        name -> Text,
        description -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::OsFormat;
    use crate::types::OsFormatType;

    product (id) {
        id -> Int4,
        id_os -> Int4,
        format -> OsFormatType,
    }
}

diesel::joinable!(operating_system -> architecture (architecture));
diesel::joinable!(operating_system -> package_manager (manager));
diesel::joinable!(operating_system_package -> operating_system (id_os));
diesel::joinable!(operating_system_package -> package (id_package));
diesel::joinable!(package -> architecture (architecture));
diesel::joinable!(package_provides -> package (id_package));
diesel::joinable!(package_requires -> package (id_package));
diesel::joinable!(package_artifact -> package (id_package));
diesel::joinable!(product -> operating_system (id_os));

diesel::allow_tables_to_appear_in_same_query!(
    architecture,
    operating_system,
    operating_system_package,
    package,
    package_dependencies,
    package_provides,
    package_requires,
    package_artifact,
    package_manager,
    product,
);
