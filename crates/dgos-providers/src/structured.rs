//! Structured output (B01-52, B01-53, AC25): request checks, the instruction used by profiles
//! without a native mode, and the host-side validation of whatever the upstream returned.
//!
//! The answer is validated here for every protocol and profile. A mismatch is
//! `structured_output_invalid` and is never retried. Schemas are compiled without remote
//! reference resolution (the `jsonschema` crate is built without its network features).

use dgos_types::{DgosError, DgosResult, ResponseFormat};
use serde_json::Value;

/// Cap on the serialized schema the host will compile and forward.
const MAX_SCHEMA_BYTES: usize = 256 * 1024;

/// `name` must be usable as an OpenAI schema name: `[A-Za-z0-9_-]{1,64}`.
fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Local refusals that need no upstream round trip: wrong `type`, bad `name`, a schema that is
/// oversized or does not compile.
pub fn check_format(format: &ResponseFormat) -> DgosResult<()> {
    if format.kind != "json_schema"
        || !valid_name(&format.name)
        || !format.schema.is_object()
        || format.schema.to_string().len() > MAX_SCHEMA_BYTES
    {
        return Err(DgosError::RequestNotSupported);
    }
    jsonschema::validator_for(&format.schema)
        .map(|_| ())
        .map_err(|_| DgosError::RequestNotSupported)
}

/// Parses `content` and checks it against the requested schema.
pub fn validate(content: &str, format: &ResponseFormat) -> DgosResult<Value> {
    let value: Value =
        serde_json::from_str(content.trim()).map_err(|_| DgosError::StructuredOutputInvalid)?;
    let validator = jsonschema::validator_for(&format.schema)
        .map_err(|_| DgosError::StructuredOutputInvalid)?;
    if validator.is_valid(&value) {
        Ok(value)
    } else {
        Err(DgosError::StructuredOutputInvalid)
    }
}

/// System instruction for profiles that have no native structured output mode. It must mention
/// JSON explicitly (some services only emit JSON when asked in words).
pub fn instruction(format: &ResponseFormat) -> String {
    format!(
        "Respond with a single JSON value that conforms to the JSON Schema below, and nothing \
         else: no prose, no code fences. Schema name: {}.\n{}",
        format.name, format.schema
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn format(schema: Value) -> ResponseFormat {
        ResponseFormat {
            kind: "json_schema".into(),
            name: "answer".into(),
            schema,
            strict: None,
        }
    }

    fn answer_schema() -> Value {
        json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"],
            "additionalProperties":false})
    }

    #[test]
    fn accepts_a_conforming_answer_and_returns_the_parsed_object() {
        let value = validate(r#" {"n": 3} "#, &format(answer_schema())).unwrap();
        assert_eq!(value, json!({"n": 3}));
    }

    #[test]
    fn rejects_empty_non_json_and_non_conforming_answers() {
        let f = format(answer_schema());
        for content in [
            "",
            "   ",
            "not json",
            "```json\n{\"n\":1}\n```",
            r#"{"n":"x"}"#,
            r#"{}"#,
            r#"[1]"#,
        ] {
            assert!(
                matches!(
                    validate(content, &f),
                    Err(DgosError::StructuredOutputInvalid)
                ),
                "{content:?}"
            );
        }
    }

    #[test]
    fn refuses_unusable_formats_before_any_request() {
        let mut f = format(answer_schema());
        assert!(check_format(&f).is_ok());
        f.name = "bad name".into();
        assert!(check_format(&f).is_err());
        f.name = "x".repeat(65);
        assert!(check_format(&f).is_err());
        f.name = String::new();
        assert!(check_format(&f).is_err());
        let mut f = format(answer_schema());
        f.kind = "json_object".into();
        assert!(check_format(&f).is_err());
        assert!(check_format(&format(json!("string"))).is_err());
        // Compiles to nothing valid.
        assert!(check_format(&format(json!({"type": "nonsense"}))).is_err());
    }

    #[test]
    fn instruction_names_json_and_carries_the_schema() {
        let text = instruction(&format(answer_schema()));
        assert!(text.contains("JSON"));
        assert!(text.contains("\"additionalProperties\":false"));
    }
}
