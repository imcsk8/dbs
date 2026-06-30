s/OsType,/OsTypeType,/
s/OsFormat,/OsFormatType,/

# Include modules
/use super::sql_types::OsType/ a \
    use crate::types::OsTypeType;

/use super::sql_types::OsFormat/ a \
    use crate::types::OsFormatType;

