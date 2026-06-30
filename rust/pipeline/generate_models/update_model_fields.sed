# Add field modifiers
/pub raw_data: Option<serde_json::Value>/ i \
    \#[diesel(serialize_as = JsonValue)]
