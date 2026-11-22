use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Schema for `config/users.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsersConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<UserConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    Member,
    Guest,
}

/// Area permissions for a user: either `"*"` (all areas) or an explicit list of `area_id`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowedAreas {
    All,
    List(Vec<String>),
}

impl Default for AllowedAreas {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

impl Serialize for AllowedAreas {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::All => serializer.serialize_str("*"),
            Self::List(areas) => areas.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for AllowedAreas {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AllowedAreasVisitor;

        impl<'de> Visitor<'de> for AllowedAreasVisitor {
            type Value = AllowedAreas;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(r#""*" or a list of area_id strings"#)
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value == "*" {
                    Ok(AllowedAreas::All)
                } else {
                    Err(E::custom(format!(
                        r#"expected "*" or a list of area_ids, got string "{value}""#
                    )))
                }
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut areas = Vec::new();
                while let Some(area) = seq.next_element::<String>()? {
                    areas.push(area);
                }
                Ok(AllowedAreas::List(areas))
            }
        }

        deserializer.deserialize_any(AllowedAreasVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiTokenConfig {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfig {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub password_hash: String,
    pub role: UserRole,
    #[serde(default)]
    pub allowed_areas: AllowedAreas,
    #[serde(default)]
    pub allow_guest_locks: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_tokens: Vec<ApiTokenConfig>,
}
