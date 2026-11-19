//! Closed, typed argument schemas for tools.
//!
//! A schema is a static list of fields; it renders to the JSON Schema a model sees (always
//! `additionalProperties: false`, the strict shape) and it validates what the model actually
//! sent. The validator is deliberately small and exact: no coercion, no defaults from `null`,
//! no unknown fields. Model output is untrusted input.

use serde_json::{Map, Number, Value, json};
use std::fmt;

/// Field names a tool may never declare. Identity and authority come from the server-side
/// context, never from model-chosen arguments (skill `ai-tool-tiers`, rule 4).
pub const FORBIDDEN_FIELD_NAMES: [&str; 9] = [
    "org",
    "org_id",
    "organization_id",
    "tenant",
    "tenant_id",
    "user_id",
    "actor_id",
    "role",
    "tier",
];

/// The type and bounds of one field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Text of 1 to `max_chars` characters, no NUL; `choices` (when set) is an exact allow-list.
    Text {
        /// Maximum length in characters.
        max_chars: usize,
        /// Allowed values, or `None` for free text.
        choices: Option<&'static [&'static str]>,
    },
    /// Whole number within `min..=max`. A JSON float (even `3.0`) is refused.
    Int {
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },
    /// `true` or `false`.
    Bool,
}

/// One named argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// Lowercase snake_case name.
    pub name: &'static str,
    /// Type and bounds.
    pub kind: Kind,
    /// Whether the model must send it.
    pub required: bool,
    /// One short sentence for the model (counts as input tokens: keep it terse).
    pub description: &'static str,
}

/// The full argument list of a tool. An empty schema means "no arguments".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgSchema {
    /// The fields, in the order shown to the model.
    pub fields: &'static [Field],
}

/// Why arguments were refused. The message is safe to show to a model: it names the field and
/// the rule, never echoes the offending value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgError {
    /// The arguments are not valid JSON.
    NotJson,
    /// The arguments are valid JSON but not an object.
    NotAnObject,
    /// The raw arguments exceed the size limit.
    TooLarge,
    /// A field that the schema does not declare.
    UnknownField(String),
    /// A required field is missing.
    Missing(&'static str),
    /// A field has the wrong JSON type, or `null`.
    WrongType(&'static str),
    /// A number outside the allowed range.
    OutOfRange(&'static str),
    /// Text that is empty, too long or contains NUL.
    BadText(&'static str),
    /// Text that is not one of the allowed choices.
    NotAllowed(&'static str),
}

impl ArgError {
    /// Stable machine code, also stored in the call record.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotJson => "args_not_json",
            Self::NotAnObject => "args_not_object",
            Self::TooLarge => "args_too_large",
            Self::UnknownField(_) => "args_unknown_field",
            Self::Missing(_) => "args_missing_field",
            Self::WrongType(_) => "args_wrong_type",
            Self::OutOfRange(_) => "args_out_of_range",
            Self::BadText(_) => "args_bad_text",
            Self::NotAllowed(_) => "args_not_allowed",
        }
    }
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson | Self::NotAnObject | Self::TooLarge => f.write_str(self.code()),
            // An unknown name is model-controlled text: cap it before it goes back to the model.
            Self::UnknownField(name) => {
                write!(
                    f,
                    "{}: {}",
                    self.code(),
                    name.chars().take(40).collect::<String>()
                )
            }
            Self::Missing(n)
            | Self::WrongType(n)
            | Self::OutOfRange(n)
            | Self::BadText(n)
            | Self::NotAllowed(n) => write!(f, "{}: {n}", self.code()),
        }
    }
}

impl std::error::Error for ArgError {}

/// Validated arguments. Only [`ArgSchema::validate`] builds one, so a tool can trust the types.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args(Map<String, Value>);

impl Args {
    /// A text argument, if it was sent.
    #[must_use]
    pub fn text(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Value::as_str)
    }

    /// An integer argument, if it was sent.
    #[must_use]
    pub fn int(&self, name: &str) -> Option<i64> {
        self.0.get(name).and_then(Value::as_i64)
    }

    /// A boolean argument, if it was sent.
    #[must_use]
    pub fn bool(&self, name: &str) -> Option<bool> {
        self.0.get(name).and_then(Value::as_bool)
    }
}

impl ArgSchema {
    /// A schema with no arguments.
    pub const NONE: ArgSchema = ArgSchema { fields: &[] };

    /// The JSON Schema shown to the model: an object, `additionalProperties: false`, every
    /// required field listed.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for field in self.fields {
            let mut prop = match field.kind {
                Kind::Text { max_chars, choices } => {
                    let mut p = json!({"type": "string", "minLength": 1, "maxLength": max_chars});
                    if let Some(choices) = choices {
                        p["enum"] = json!(choices);
                    }
                    p
                }
                Kind::Int { min, max } => {
                    json!({"type": "integer", "minimum": min, "maximum": max})
                }
                Kind::Bool => json!({"type": "boolean"}),
            };
            prop["description"] = json!(field.description);
            properties.insert(field.name.to_owned(), prop);
            if field.required {
                required.push(field.name);
            }
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }

    /// Parses and validates the raw argument string of a tool call. Empty text means `{}`
    /// (some models send nothing for a tool without arguments).
    ///
    /// # Errors
    /// An [`ArgError`] naming the first violated rule.
    pub fn validate(&self, raw: &str, max_bytes: usize) -> Result<Args, ArgError> {
        if raw.len() > max_bytes {
            return Err(ArgError::TooLarge);
        }
        let value: Value = if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(raw).map_err(|_| ArgError::NotJson)?
        };
        let Value::Object(map) = value else {
            return Err(ArgError::NotAnObject);
        };
        if let Some(unknown) = map
            .keys()
            .find(|k| !self.fields.iter().any(|f| f.name == k.as_str()))
        {
            return Err(ArgError::UnknownField(unknown.clone()));
        }
        for field in self.fields {
            match map.get(field.name) {
                None if field.required => return Err(ArgError::Missing(field.name)),
                None => {}
                Some(value) => check(field, value)?,
            }
        }
        Ok(Args(map))
    }

    /// Whether any field has a name in [`FORBIDDEN_FIELD_NAMES`], or a malformed name or bounds.
    /// Used by the registry; returns the offending field name.
    #[must_use]
    pub fn first_invalid_field(&self) -> Option<&'static str> {
        for (i, field) in self.fields.iter().enumerate() {
            let name = field.name;
            let well_formed = !name.is_empty()
                && name.len() <= 40
                && name.starts_with(|c: char| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            let bounds_ok = match field.kind {
                Kind::Text { max_chars, choices } => {
                    max_chars > 0 && choices.is_none_or(|c| !c.is_empty())
                }
                Kind::Int { min, max } => min <= max,
                Kind::Bool => true,
            };
            let duplicate = self.fields[..i].iter().any(|f| f.name == name);
            if !well_formed || !bounds_ok || duplicate || FORBIDDEN_FIELD_NAMES.contains(&name) {
                return Some(name);
            }
        }
        None
    }
}

fn check(field: &Field, value: &Value) -> Result<(), ArgError> {
    let name = field.name;
    match field.kind {
        Kind::Text { max_chars, choices } => {
            let Value::String(text) = value else {
                return Err(ArgError::WrongType(name));
            };
            if text.is_empty() || text.contains('\0') || text.chars().count() > max_chars {
                return Err(ArgError::BadText(name));
            }
            if choices.is_some_and(|c| !c.contains(&text.as_str())) {
                return Err(ArgError::NotAllowed(name));
            }
        }
        Kind::Int { min, max } => {
            let Value::Number(number) = value else {
                return Err(ArgError::WrongType(name));
            };
            let Some(n) = as_exact_i64(number) else {
                return Err(ArgError::WrongType(name));
            };
            if n < min || n > max {
                return Err(ArgError::OutOfRange(name));
            }
        }
        Kind::Bool => {
            if !value.is_boolean() {
                return Err(ArgError::WrongType(name));
            }
        }
    }
    Ok(())
}

/// `Some` only for a JSON integer that fits `i64` (no float, no `3.0`, no overflow).
fn as_exact_i64(number: &Number) -> Option<i64> {
    number.as_i64()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: ArgSchema = ArgSchema {
        fields: &[
            Field {
                name: "service",
                kind: Kind::Text {
                    max_chars: 8,
                    choices: None,
                },
                required: true,
                description: "Service name.",
            },
            Field {
                name: "limit",
                kind: Kind::Int { min: 1, max: 50 },
                required: false,
                description: "Rows.",
            },
            Field {
                name: "mode",
                kind: Kind::Text {
                    max_chars: 10,
                    choices: Some(&["brief", "full"]),
                },
                required: false,
                description: "Detail.",
            },
            Field {
                name: "all",
                kind: Kind::Bool,
                required: false,
                description: "All.",
            },
        ],
    };

    fn run(raw: &str) -> Result<Args, ArgError> {
        SCHEMA.validate(raw, 1024)
    }

    #[test]
    fn accepts_a_valid_call_and_reads_typed_values() {
        let a =
            run(r#"{"service":"nginx","limit":5,"mode":"full","all":true}"#).unwrap_or_default();
        assert_eq!(a.text("service"), Some("nginx"));
        assert_eq!(a.int("limit"), Some(5));
        assert_eq!(a.bool("all"), Some(true));
        assert_eq!(a.text("missing"), None);
    }

    #[test]
    fn refuses_every_kind_of_bad_input() {
        let cases = [
            ("not json", ArgError::NotJson),
            ("[1]", ArgError::NotAnObject),
            ("\"x\"", ArgError::NotAnObject),
            (r#"{"limit":3}"#, ArgError::Missing("service")),
            (
                r#"{"service":"a","org_id":"o"}"#,
                ArgError::UnknownField("org_id".into()),
            ),
            (r#"{"service":1}"#, ArgError::WrongType("service")),
            (r#"{"service":null}"#, ArgError::WrongType("service")),
            (r#"{"service":""}"#, ArgError::BadText("service")),
            (r#"{"service":"abcdefghi"}"#, ArgError::BadText("service")),
            (r#"{"service":"a\u0000b"}"#, ArgError::BadText("service")),
            (
                r#"{"service":"a","limit":3.0}"#,
                ArgError::WrongType("limit"),
            ),
            (
                r#"{"service":"a","limit":"3"}"#,
                ArgError::WrongType("limit"),
            ),
            (
                r#"{"service":"a","limit":0}"#,
                ArgError::OutOfRange("limit"),
            ),
            (
                r#"{"service":"a","limit":51}"#,
                ArgError::OutOfRange("limit"),
            ),
            (
                r#"{"service":"a","limit":99999999999999999999}"#,
                ArgError::WrongType("limit"),
            ),
            (
                r#"{"service":"a","mode":"other"}"#,
                ArgError::NotAllowed("mode"),
            ),
            (
                r#"{"service":"a","all":"true"}"#,
                ArgError::WrongType("all"),
            ),
        ];
        for (raw, expected) in cases {
            assert_eq!(run(raw), Err(expected), "input: {raw}");
        }
    }

    #[test]
    fn size_limit_is_checked_before_parsing() {
        assert_eq!(
            SCHEMA.validate(&"x".repeat(2000), 1024),
            Err(ArgError::TooLarge)
        );
    }

    #[test]
    fn empty_arguments_mean_an_empty_object() {
        assert!(ArgSchema::NONE.validate("", 1024).is_ok());
        assert!(ArgSchema::NONE.validate("  ", 1024).is_ok());
        assert_eq!(
            ArgSchema::NONE.validate(r#"{"a":1}"#, 1024),
            Err(ArgError::UnknownField("a".into()))
        );
    }

    #[test]
    fn json_schema_is_strict_and_lists_required_fields() {
        let json = SCHEMA.to_json();
        assert_eq!(json["additionalProperties"], false);
        assert_eq!(json["required"], json!(["service"]));
        assert_eq!(json["properties"]["limit"]["maximum"], 50);
        assert_eq!(json["properties"]["mode"]["enum"], json!(["brief", "full"]));
    }

    #[test]
    fn error_text_never_echoes_values_and_caps_unknown_names() {
        let long = format!(r#"{{"service":"a","{}":1}}"#, "z".repeat(500));
        let msg = run(&long).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(msg.len() < 80, "{msg}");
    }

    #[test]
    fn rejects_forbidden_malformed_and_duplicate_fields() {
        const BAD: [ArgSchema; 4] = [
            ArgSchema {
                fields: &[Field {
                    name: "org_id",
                    kind: Kind::Bool,
                    required: false,
                    description: "",
                }],
            },
            ArgSchema {
                fields: &[Field {
                    name: "Bad-Name",
                    kind: Kind::Bool,
                    required: false,
                    description: "",
                }],
            },
            ArgSchema {
                fields: &[
                    Field {
                        name: "a",
                        kind: Kind::Bool,
                        required: false,
                        description: "",
                    },
                    Field {
                        name: "a",
                        kind: Kind::Bool,
                        required: false,
                        description: "",
                    },
                ],
            },
            ArgSchema {
                fields: &[Field {
                    name: "n",
                    kind: Kind::Int { min: 5, max: 1 },
                    required: false,
                    description: "",
                }],
            },
        ];
        for schema in BAD {
            assert!(schema.first_invalid_field().is_some(), "{schema:?}");
        }
        assert_eq!(SCHEMA.first_invalid_field(), None);
        for name in FORBIDDEN_FIELD_NAMES {
            // Leaked on purpose: the schema needs a 'static slice and this is a test.
            let fields: &'static [Field] = Box::leak(Box::new([Field {
                name,
                kind: Kind::Bool,
                required: false,
                description: "",
            }]));
            assert_eq!(ArgSchema { fields }.first_invalid_field(), Some(name));
        }
    }
}
