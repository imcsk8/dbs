// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "os_format"))]
    pub struct OsFormat;

    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "os_type"))]
    pub struct OsType;
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
    package (id) {
        id -> Int4,
        name -> Text,
        version -> Text,
        release -> Text,
        architecture -> Int4,
        package_size -> Text,
        source -> Text,
        repository -> Text,
        summary -> Text,
        url -> Text,
        license -> Text,
        description -> Text,
        in_repo -> Nullable<Bool>,
        created -> Nullable<Bool>,
        vulnerable -> Nullable<Bool>,
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
diesel::joinable!(product -> operating_system (id_os));

diesel::allow_tables_to_appear_in_same_query!(
    architecture,
    operating_system,
    operating_system_package,
    package,
    package_dependencies,
    package_manager,
    product,
);
