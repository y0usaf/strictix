//! Integration tests for the M8 minimal JSON parser.
//!
//! Covers every value kind, all string escapes (including surrogate
//! pairs), nesting, object lookup and insertion order, whitespace
//! tolerance, the required error cases, an options.json-shaped document,
//! and a generated 100-key document.

use strictix_core::json::{JsonError, JsonValue};

/// Parse successfully or panic with the input as context.
fn ok(input: &str) -> JsonValue {
    JsonValue::parse(input).unwrap_or_else(|e| panic!("{input:?} should parse, got {e:?}"))
}

/// Parse to an error (panic on success).
fn err(input: &str) -> JsonError {
    JsonValue::parse(input).unwrap_err()
}

#[test]
fn scalar_values() {
    assert_eq!(JsonValue::parse("null"), Ok(JsonValue::Null));
    assert_eq!(JsonValue::parse("true"), Ok(JsonValue::Bool(true)));
    assert_eq!(JsonValue::parse("false"), Ok(JsonValue::Bool(false)));
    assert_eq!(JsonValue::parse("0"), Ok(JsonValue::Number(0.0)));
    assert_eq!(JsonValue::parse("-1"), Ok(JsonValue::Number(-1.0)));
    assert_eq!(JsonValue::parse("3.14"), Ok(JsonValue::Number(3.14)));
    assert_eq!(JsonValue::parse("-2.5e3"), Ok(JsonValue::Number(-2500.0)));
    assert_eq!(JsonValue::parse("1e10"), Ok(JsonValue::Number(1e10)));
    assert_eq!(JsonValue::parse("0.5"), Ok(JsonValue::Number(0.5)));
    // Accessors on scalars.
    assert_eq!(ok("true").as_bool(), Some(true));
    assert_eq!(ok("false").as_bool(), Some(false));
    assert_eq!(ok("3").as_bool(), None);
    assert_eq!(ok("\"hi\"").as_str(), Some("hi"));
    assert_eq!(ok("1").as_str(), None);
}

#[test]
fn string_escapes() {
    assert_eq!(
        JsonValue::parse(r#""a\"b""#),
        Ok(JsonValue::String("a\"b".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""a\\b""#),
        Ok(JsonValue::String("a\\b".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""a\/b""#),
        Ok(JsonValue::String("a/b".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""a\bb""#),
        Ok(JsonValue::String("a\u{8}b".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""a\fb""#),
        Ok(JsonValue::String("a\u{c}b".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""a\nb""#),
        Ok(JsonValue::String("a\nb".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""a\rb""#),
        Ok(JsonValue::String("a\rb".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""a\tb""#),
        Ok(JsonValue::String("a\tb".to_owned()))
    );
}

#[test]
fn string_unicode() {
    assert_eq!(
        JsonValue::parse(r#""\u0041""#),
        Ok(JsonValue::String("A".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""\u00e9""#),
        Ok(JsonValue::String("\u{e9}".to_owned()))
    );
    // Surrogate pair U+D83D U+DE00 -> U+1F600 (grinning face).
    assert_eq!(
        JsonValue::parse(r#""\ud83d\ude00""#),
        Ok(JsonValue::String("\u{1f600}".to_owned()))
    );
    // Raw multi-byte UTF-8 passes through unchanged.
    assert_eq!(
        JsonValue::parse("\"h\u{e9}llo\""),
        Ok(JsonValue::String("h\u{e9}llo".to_owned()))
    );
}

#[test]
fn nested_structures() {
    assert_eq!(JsonValue::parse("[]"), Ok(JsonValue::Array(vec![])));
    assert_eq!(JsonValue::parse("{}"), Ok(JsonValue::Object(vec![])));
    let nested = ok(r#"[[1, [2]], {"a": [true, null]}]"#);
    assert_eq!(
        nested,
        JsonValue::Array(vec![
            JsonValue::Array(vec![
                JsonValue::Number(1.0),
                JsonValue::Array(vec![JsonValue::Number(2.0)])
            ]),
            JsonValue::Object(vec![(
                "a".to_owned(),
                JsonValue::Array(vec![JsonValue::Bool(true), JsonValue::Null])
            )])
        ])
    );
}

#[test]
fn object_lookup_and_order() {
    let v = ok(r#"{"b": 1, "a": 2, "c": 3}"#);
    let obj = v.as_object().expect("object");
    let keys: Vec<&str> = obj.iter().map(|(k, _)| k.as_str()).collect();
    // Insertion order is preserved.
    assert_eq!(keys, vec!["b", "a", "c"]);
    assert_eq!(v.get("a"), Some(&JsonValue::Number(2.0)));
    assert_eq!(v.get("c"), Some(&JsonValue::Number(3.0)));
    assert_eq!(v.get("missing"), None);
    // get on a non-object returns None.
    assert_eq!(JsonValue::Null.get("x"), None);
}

#[test]
fn duplicate_keys_kept() {
    let v = ok(r#"{"a": 1, "a": 2}"#);
    let obj = v.as_object().expect("object");
    assert_eq!(obj.len(), 2);
    // get() returns the first entry.
    assert_eq!(v.get("a"), Some(&JsonValue::Number(1.0)));
}

#[test]
fn whitespace_tolerance() {
    assert_eq!(JsonValue::parse("  {  }  "), Ok(JsonValue::Object(vec![])));
    assert_eq!(
        JsonValue::parse("\n\t {\"a\": 1} \r\n"),
        Ok(JsonValue::Object(vec![(
            "a".to_owned(),
            JsonValue::Number(1.0)
        )]))
    );
    assert_eq!(
        JsonValue::parse(" \r\n [ 1 , 2 ] \t "),
        Ok(JsonValue::Array(vec![
            JsonValue::Number(1.0),
            JsonValue::Number(2.0)
        ]))
    );
}

#[test]
fn numbers_in_strings_stay_strings() {
    assert_eq!(
        JsonValue::parse(r#""123""#),
        Ok(JsonValue::String("123".to_owned()))
    );
    assert_eq!(
        JsonValue::parse(r#""true""#),
        Ok(JsonValue::String("true".to_owned()))
    );
}

#[test]
fn errors() {
    // Trailing junk after a value.
    assert_eq!(
        err("1 x"),
        JsonError {
            message: "trailing content".to_owned(),
            offset: 2
        }
    );
    // Unclosed structures.
    assert_eq!(
        err("{"),
        JsonError {
            message: "unterminated object".to_owned(),
            offset: 1
        }
    );
    assert_eq!(
        err("["),
        JsonError {
            message: "unterminated array".to_owned(),
            offset: 1
        }
    );
    assert_eq!(
        err("{\"a\": 1"),
        JsonError {
            message: "unterminated object".to_owned(),
            offset: 7
        }
    );
    assert_eq!(
        err("[1, 2"),
        JsonError {
            message: "unterminated array".to_owned(),
            offset: 5
        }
    );
    // Unclosed string.
    assert_eq!(
        err("\"abc"),
        JsonError {
            message: "unterminated string".to_owned(),
            offset: 4
        }
    );
    // Bad escape.
    assert_eq!(
        err(r#""a\q""#),
        JsonError {
            message: "invalid escape".to_owned(),
            offset: 3
        }
    );
    // Invalid numbers.
    assert_eq!(
        err("01"),
        JsonError {
            message: "invalid number".to_owned(),
            offset: 1
        }
    );
    assert_eq!(
        err("1."),
        JsonError {
            message: "invalid number".to_owned(),
            offset: 2
        }
    );
    assert_eq!(
        err(".5"),
        JsonError {
            message: "invalid number".to_owned(),
            offset: 0
        }
    );
    assert_eq!(
        err("1e"),
        JsonError {
            message: "invalid number".to_owned(),
            offset: 2
        }
    );
    assert_eq!(
        err("1e999"),
        JsonError {
            message: "invalid number".to_owned(),
            offset: 5
        }
    );
    // Bare NaN / Infinity are not JSON numbers.
    assert_eq!(
        err("NaN"),
        JsonError {
            message: "unexpected character".to_owned(),
            offset: 0
        }
    );
    assert_eq!(
        err("Infinity"),
        JsonError {
            message: "unexpected character".to_owned(),
            offset: 0
        }
    );
    // Unpaired surrogates.
    assert_eq!(
        err(r#""\ud800""#),
        JsonError {
            message: "unpaired surrogate".to_owned(),
            offset: 7
        }
    );
    assert_eq!(
        err(r#""\udc00""#),
        JsonError {
            message: "unpaired surrogate".to_owned(),
            offset: 7
        }
    );
    // Missing colon / comma.
    assert_eq!(
        err(r#"{"a" 1}"#),
        JsonError {
            message: "expected ':'".to_owned(),
            offset: 5
        }
    );
    assert_eq!(
        err(r#"{"a": 1 "b": 2}"#),
        JsonError {
            message: "expected ',' or '}'".to_owned(),
            offset: 8
        }
    );
    assert_eq!(
        err("[1 2]"),
        JsonError {
            message: "expected ',' or ']'".to_owned(),
            offset: 3
        }
    );
    // Empty and whitespace-only input.
    assert_eq!(
        err(""),
        JsonError {
            message: "unexpected end of input".to_owned(),
            offset: 0
        }
    );
    assert_eq!(
        err("   "),
        JsonError {
            message: "unexpected end of input".to_owned(),
            offset: 3
        }
    );
}

#[test]
fn options_json_shape() {
    let doc = r#"{"options": {"services.foo.enable": {"type": "boolean", "description": "x"}}}"#;
    let v = ok(doc);
    let options = v.get("options").expect("options key");
    let option = options.get("services.foo.enable").expect("option path");
    let obj = option.as_object().expect("option object");
    assert_eq!(obj.len(), 2);
    assert_eq!(
        option.get("type").and_then(JsonValue::as_str),
        Some("boolean")
    );
    assert_eq!(
        option.get("description").and_then(JsonValue::as_str),
        Some("x")
    );
}

#[test]
fn large_document() {
    // Generate a 100-key object programmatically and parse it back.
    let mut body = String::from("{");
    for i in 0..100 {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!("\"key{i}\": {i}"));
    }
    body.push('}');
    let v = ok(&body);
    let obj = v.as_object().expect("object");
    assert_eq!(obj.len(), 100);
    assert_eq!(obj.first().map(|(k, _)| k.as_str()), Some("key0"));
    assert_eq!(obj.last().map(|(k, _)| k.as_str()), Some("key99"));
    assert_eq!(v.get("key0"), Some(&JsonValue::Number(0.0)));
    assert_eq!(v.get("key42"), Some(&JsonValue::Number(42.0)));
    assert_eq!(v.get("key99"), Some(&JsonValue::Number(99.0)));
}
