#![allow(clippy::err_expect, clippy::ok_expect)]
use coded_error::ErrorClass;
use op_status::{
    assert_allowed_keys, check_component, AttrValue, Component, State, MAX_ATTRS,
    MAX_ATTR_KEY_BYTES, MAX_ATTR_VALUE_BYTES,
};

const ALLOWED: &[&str] = &["count", "name"];

fn code_and_message(err: op_status::AttrError) -> (&'static str, ErrorClass, String) {
    let any = coded_error::into_anyhow(err);
    let facts = coded_error::facts_anyhow(&any);
    (facts.code, facts.class, format!("{any:#}"))
}

fn base() -> Component {
    Component::new("room", "r1", State::Ok)
}

#[test]
fn unlisted_attr_key_is_refused_naming_the_key() {
    let ok = base().attr("count", 1i64).attr("name", "n");
    assert!(check_component(&ok, ALLOWED).is_ok());

    let bad = base().attr("count", 1i64).attr("secret_path", "/etc/x");
    let err = check_component(&bad, ALLOWED).err().expect("must refuse");
    let (code, class, msg) = code_and_message(err);
    assert_eq!(code, "op_status.attr_not_allowed");
    assert_eq!(class, ErrorClass::Logical);
    assert!(msg.contains("secret_path"), "{msg}");
}

#[test]
fn attr_limits_on_count_key_and_value_length() {
    // Count: 32 fine, 33 too many.
    let owned: Vec<String> = (0..=MAX_ATTRS).map(|i| format!("k{i:02}")).collect();
    let allowed: Vec<&str> = owned.iter().map(String::as_str).collect();
    let mut c = base();
    for k in owned.iter().take(MAX_ATTRS) {
        c = c.attr(k.clone(), 1i64);
    }
    assert_eq!(MAX_ATTRS, 32);
    assert!(check_component(&c, &allowed).is_ok());
    let c33 = c.attr(owned[MAX_ATTRS].clone(), 1i64);
    let (code, class, _) = code_and_message(check_component(&c33, &allowed).err().unwrap());
    assert_eq!(code, "op_status.attr_too_many");
    assert_eq!(class, ErrorClass::Logical);

    // Key length: 64 bytes fine, 65 too long.
    assert_eq!(MAX_ATTR_KEY_BYTES, 64);
    let k64 = "k".repeat(64);
    let k65 = "k".repeat(65);
    let allowed = [k64.as_str(), k65.as_str()];
    assert!(check_component(&base().attr(k64.clone(), 1i64), &allowed).is_ok());
    let (code, class, _) = code_and_message(
        check_component(&base().attr(k65.clone(), 1i64), &allowed)
            .err()
            .unwrap(),
    );
    assert_eq!(code, "op_status.attr_key_too_long");
    assert_eq!(class, ErrorClass::Logical);

    // Value length: 256-byte Str fine, 257 too long; non-Str never limited.
    assert_eq!(MAX_ATTR_VALUE_BYTES, 256);
    assert!(check_component(&base().attr("name", "v".repeat(256)), ALLOWED).is_ok());
    let (code, class, msg) = code_and_message(
        check_component(&base().attr("name", "v".repeat(257)), ALLOWED)
            .err()
            .unwrap(),
    );
    assert_eq!(code, "op_status.attr_value_too_long");
    assert_eq!(class, ErrorClass::Logical);
    assert!(msg.contains("name"), "{msg}");
    let mut c = base();
    c = c.attr("count", AttrValue::Int(i64::MAX));
    c = c.attr("name", AttrValue::Float(f64::MAX));
    assert!(check_component(&c, ALLOWED).is_ok());
    assert!(check_component(&base().attr("count", true), ALLOWED).is_ok());
}

#[test]
#[should_panic(expected = "leaky_key")]
fn assert_allowed_keys_fails_on_an_unlisted_key() {
    // Passing cases must not panic (a panic here would not contain the key).
    assert_allowed_keys(&["a", "b"], ["a", "b", "a"]);
    assert_allowed_keys(&["a"], std::iter::empty::<&str>());
    // Failing case: panics naming the key.
    assert_allowed_keys(&["a", "b"], ["a", "leaky_key"]);
}
