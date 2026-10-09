//! The generated JSON Schema must accept every model that the deserialisers accept.

use jsonschema::Validator;
use pywr_v1_schema::PywrModel;
use pywr_v1_schema::json_schema::{CustomTypes, model_schema, network_schema};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

/// Models that are network fragments pulled in by another model's `includes`, not whole models.
const NETWORK_FRAGMENTS: [&str; 1] = ["extra2.json"];

/// Models in which a core node or parameter is not modelled by the crate, so the deserialisers read
/// it as a custom one. Strict validation reports them; each is expected to fail it.
const READ_AS_CUSTOM: [&str; 2] = [
    // `interpolatedvolume` with `volumes` and `values` read from external data.
    "reservoir_evaporation_areafromfile.json",
    // A storage node whose `initial_volume` is a table reference.
    "reservoir_initial_vol_from_table.json",
];

fn models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("models")
}

fn validator_for(schema: schemars::Schema) -> Validator {
    let schema = serde_json::to_value(schema).expect("schema serialises");
    jsonschema::validator_for(&schema).expect("the generated schema is a valid JSON Schema")
}

fn read_model(name: &str) -> Value {
    let path = models_dir().join(name);
    let data = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&data).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// The file names of every whole model in the test models directory.
fn model_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(models_dir())
        .expect("models directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            path.file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| !NETWORK_FRAGMENTS.contains(&name.as_str()))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no test models found");
    names
}

fn errors(validator: &Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}

#[test]
fn every_test_model_validates() {
    let validator = validator_for(model_schema(CustomTypes::Any));
    let failures: Vec<String> = model_names()
        .into_iter()
        .filter_map(|name| {
            let errors = errors(&validator, &read_model(&name));
            (!errors.is_empty()).then(|| format!("{name}: {}", errors.join("; ")))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} models failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn network_fragments_validate_as_networks() {
    let validator = validator_for(network_schema(CustomTypes::Any));
    for name in NETWORK_FRAGMENTS {
        assert_eq!(
            errors(&validator, &read_model(name)),
            Vec::<String>::new(),
            "{name}"
        );
    }
}

/// Without the custom fallback the core definitions themselves are checked, which catches fields
/// the schema does not describe (a serde `alias`, for example) that the lenient schema hides.
#[test]
fn core_definitions_in_test_models_are_valid() {
    let validator = validator_for(model_schema(CustomTypes::NonCoreOnly));
    for name in model_names() {
        let errors = errors(&validator, &read_model(&name));
        if READ_AS_CUSTOM.contains(&name.as_str()) {
            assert!(
                !errors.is_empty(),
                "{name} is no longer read as custom: remove it from READ_AS_CUSTOM"
            );
        } else {
            assert!(errors.is_empty(), "{name}: {}", errors.join("; "));
        }
    }
}

/// A schema that accepted everything would pass the tests above, so check that breaking a model in
/// each of several ways makes it fail.
#[test]
fn broken_models_are_rejected() {
    let validator = validator_for(model_schema(CustomTypes::Any));
    let valid = read_model("river1.json");
    assert!(
        validator.is_valid(&valid),
        "river1.json should be valid before it is broken"
    );

    let mut renamed_timestepper = valid.clone();
    let object = renamed_timestepper
        .as_object_mut()
        .expect("a model is an object");
    let timestepper = object
        .remove("timestepper")
        .expect("river1.json has a timestepper");
    object.insert("timestep".to_string(), timestepper);
    assert!(
        !validator.is_valid(&renamed_timestepper),
        "renamed `timestepper` key must be rejected"
    );

    let mut unnamed_node = valid.clone();
    let node = unnamed_node["nodes"][0]
        .as_object_mut()
        .expect("a node is an object");
    let name = node.remove("name").expect("a node has a name");
    node.insert("nme".to_string(), name);
    assert!(
        !validator.is_valid(&unnamed_node),
        "renamed node `name` key must be rejected"
    );

    let mut short_edge = valid.clone();
    short_edge["edges"][0] = json!(["only-one-node"]);
    assert!(
        !validator.is_valid(&short_edge),
        "an edge with one entry must be rejected"
    );

    let mut long_slice = valid.clone();
    long_slice["scenarios"] = json!([{"name": "s", "size": 4, "slice": [0, 1, 2, 3]}]);
    assert!(
        !validator.is_valid(&long_slice),
        "a scenario slice of four entries must be rejected"
    );
}

/// Custom nodes and parameters stay valid in both modes: a type the crate does not know is not an
/// error.
#[test]
fn custom_nodes_and_parameters_are_accepted() {
    for custom_types in [CustomTypes::Any, CustomTypes::NonCoreOnly] {
        let validator = validator_for(model_schema(custom_types));
        let mut model = read_model("river1.json");
        model["nodes"]
            .as_array_mut()
            .expect("nodes")
            .push(json!({"name": "special", "type": "MyCustomNode", "anything": [1, 2, 3]}));
        model["parameters"] = json!({
            "mine": {"type": "MyCustomParameter", "anything": {"a": 1}},
            "by_alias": {"type": "constantparameter", "value": 1.0}
        });
        assert_eq!(
            errors(&validator, &model),
            Vec::<String>::new(),
            "{custom_types:?}"
        );
    }
}

/// An invalid core definition is reported only in strict mode.
#[test]
fn strict_mode_reports_invalid_core_parameters() {
    let mut model = read_model("river1.json");
    model["parameters"] =
        json!({"p": {"type": "aggregated", "agg_func": "bogus", "parameters": []}});

    assert!(validator_for(model_schema(CustomTypes::Any)).is_valid(&model));
    assert!(!validator_for(model_schema(CustomTypes::NonCoreOnly)).is_valid(&model));
}

/// Properties and values that serde reads under another name must validate.
#[test]
fn serde_aliases_are_accepted() {
    let validator = validator_for(model_schema(CustomTypes::NonCoreOnly));
    let mut model = read_model("river1.json");
    model["parameters"] = json!({
        "index": {"type": "constant", "values": 1.0},
        "array": {"type": "indexedarray", "index_parameter": "index", "params": [1.0, 2.0]},
        "threshold": {"type": "parameterthreshold", "parameter": "index", "threshold": 1.0, "predicate": ">="}
    });
    assert_eq!(errors(&validator, &model), Vec::<String>::new());

    // The deserialiser reads every one of them as a core parameter, not a custom one.
    let parsed: PywrModel = serde_json::from_value(model).expect("the model deserialises");
    let parameters = parsed.network.parameters.expect("the model has parameters");
    assert!(parameters.iter().all(|p| !p.is_custom()));
}

/// A model whose `parameters` map holds `parameters`.
fn model_with_parameters(parameters: Value) -> Value {
    let mut model = read_model("river1.json");
    model["parameters"] = parameters;
    model
}

/// The name-keyed `parameters` map reads `type` case-insensitively, with or without a `parameter`
/// suffix, so strict mode must check the definition however the type is spelt.
#[test]
fn strict_mode_reports_invalid_core_parameters_under_any_type_spelling() {
    let lenient = validator_for(model_schema(CustomTypes::Any));
    let strict = validator_for(model_schema(CustomTypes::NonCoreOnly));
    for ty in [
        "constant",
        "Constant",
        "CONSTANT",
        "cOnStAnT",
        "constantparameter",
        "ConstantParameter",
        "CONSTANTPARAMETER",
        "constantParameter",
        "constantparameterparameter",
    ] {
        let invalid = model_with_parameters(json!({"p": {"type": ty, "value": "x"}}));
        assert!(lenient.is_valid(&invalid), "{ty}: lenient");
        assert!(!strict.is_valid(&invalid), "{ty}: strict");

        let valid = model_with_parameters(json!({"p": {"type": ty, "value": 1.0}}));
        assert_eq!(errors(&strict, &valid), Vec::<String>::new(), "{ty}");
    }
}

/// A type that only differs by case from a core one is still a custom type when it names none.
#[test]
fn strict_mode_accepts_custom_parameters_beside_case_folded_core_ones() {
    let strict = validator_for(model_schema(CustomTypes::NonCoreOnly));
    let model = model_with_parameters(json!({
        "a": {"type": "MyConstant", "value": "x"},
        "b": {"type": "constants", "value": "x"},
        "c": {"type": "parameter", "value": "x"}
    }));
    assert_eq!(errors(&strict, &model), Vec::<String>::new());
}

/// The Kelvin sign lower-cases to `k`, so the deserialiser reads it as a core `WeeklyProfile`.
#[test]
fn strict_mode_follows_unicode_lower_casing() {
    let strict = validator_for(model_schema(CustomTypes::NonCoreOnly));
    let model = model_with_parameters(json!({
        "w": {"type": "Wee\u{212A}lyProfile", "values": "x"}
    }));
    assert!(!strict.is_valid(&model));
}

/// Nodes and inline parameters match `type` exactly, so a differently-cased spelling is a custom
/// definition there, in strict mode as in the deserialiser.
#[test]
fn nodes_and_inline_parameters_match_type_exactly() {
    let strict = validator_for(model_schema(CustomTypes::NonCoreOnly));

    let mut model = read_model("river1.json");
    model["nodes"][0] = json!({"name": "n", "type": "INPUT", "max_flow": "not-a-number"});
    assert!(strict.is_valid(&model));
    let parsed: PywrModel = serde_json::from_value(model).expect("the model deserialises");
    let nodes = parsed.network.nodes.expect("the model has nodes");
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, pywr_v1_schema::nodes::Node::Custom(_)))
    );

    let inline = model_with_parameters(json!({
        "outer": {"type": "max", "parameter": {"type": "CONSTANT", "value": "x"}, "threshold": 1.0}
    }));
    assert!(strict.is_valid(&inline));
    let invalid_inline = model_with_parameters(json!({
        "outer": {"type": "max", "parameter": {"type": "constant", "value": "x"}, "threshold": 1.0}
    }));
    assert!(!strict.is_valid(&invalid_inline));
}

/// `HydroPowerTarget` is read as a core parameter, so strict mode checks its definition, here
/// with the `target` of the shipped model replaced by a value of the wrong type.
#[test]
fn strict_mode_checks_the_differently_cased_core_parameter_of_a_test_model() {
    let strict = validator_for(model_schema(CustomTypes::NonCoreOnly));
    let mut model = read_model("hydropower_target_example.json");
    assert!(strict.is_valid(&model));

    let parameter = model["parameters"]
        .as_object_mut()
        .expect("parameters")
        .values_mut()
        .find(|p| p["type"] == "HydroPowerTarget")
        .expect("the model has a HydroPowerTarget parameter");
    parameter["turbine_elevation"] = json!(["not", "a", "number"]);
    assert!(!strict.is_valid(&model));
}
