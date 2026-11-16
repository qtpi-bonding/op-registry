use op_registry::{Access, Descriptor};
use serde_json::json;

#[test]
fn descriptor_is_serializable_data_with_stable_field_order() {
    let descriptor = Descriptor::new("action", "get")
        .description("Gets an action")
        .access(Access::reads().with_network())
        .streaming(true)
        .input_schema(json!({"type": "object"}))
        .output_schema(json!({"type": "string"}))
        .renders_text(true)
        .exit_code_on_ok(true);
    let serialized = serde_json::to_string(&descriptor).unwrap();
    let keys = [
        "name",
        "noun",
        "verb",
        "description",
        "access",
        "audience",
        "streaming",
        "input_schema",
        "output_schema",
        "renders_text",
        "exit_code_on_ok",
        "requires",
        "hidden",
        "since",
        "deprecated",
        "aliases",
    ];
    let positions: Vec<_> = keys
        .iter()
        .map(|key| serialized.find(&format!("\"{key}\"")).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

    let round_trip: Descriptor = serde_json::from_str(&serialized).unwrap();
    assert_eq!(descriptor, round_trip);
    assert_eq!(
        serde_json::to_value(Access::reads().with_network()).unwrap(),
        json!({
            "reads": true,
            "writes": false,
            "network": true,
            "spawns": false,
            "destructive": false,
        })
    );
}

#[test]
fn declared_fields_serialise_with_the_descriptor() {
    let descriptor = Descriptor::new("action", "get")
        .requires(vec!["a".into(), "b".into()])
        .hidden(true)
        .since("0.2")
        .deprecated("use x")
        .aliases(vec!["g".into()]);
    let value = serde_json::to_value(descriptor).unwrap();
    assert_eq!(value["requires"], json!(["a", "b"]));
    assert_eq!(value["hidden"], json!(true));
    assert_eq!(value["since"], json!("0.2"));
    assert_eq!(value["deprecated"], json!("use x"));
    assert_eq!(value["aliases"], json!(["g"]));

    let default = serde_json::to_value(Descriptor::default()).unwrap();
    assert_eq!(default["requires"], json!([]));
    assert_eq!(default["hidden"], json!(false));
    assert_eq!(default["since"], json!(null));
    assert_eq!(default["deprecated"], json!(null));
    assert_eq!(default["aliases"], json!([]));
}

#[test]
fn name_is_derived_noun_verb_unless_explicit() {
    let descriptor = Descriptor::new("action", "get");
    assert_eq!(descriptor.name, "action_get");
    assert_eq!(descriptor.cli_name(), "action get");

    let descriptor = descriptor.name("custom");
    assert_eq!(descriptor.name, "custom");
    assert_eq!(descriptor.cli_name(), "custom");
}

#[test]
fn an_explicit_name_survives_a_round_trip() {
    let descriptor = Descriptor::new("action", "get").name("custom");
    let text = serde_json::to_string(&descriptor).unwrap();
    let back: Descriptor = serde_json::from_str(&text).unwrap();
    assert_eq!(back.cli_name(), "custom");
}

#[test]
fn is_path_true_for_a_derived_name_false_for_an_explicit_one() {
    assert!(Descriptor::new("agent dispatch-target", "set").is_path());
    assert!(Descriptor::new("item", "get").is_path());
    assert!(!Descriptor::new("system", "ping").name("ping").is_path());
}
