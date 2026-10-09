//! JSON Schema documents for the Pywr v1 model formats, derived from the Rust types.
//!
//! The schemas describe what the deserialisers in this crate accept. Nodes and parameters fall back
//! to a custom type when a core definition fails to deserialise, so a core node or parameter with a
//! misspelt field is still valid as a custom one; the schemas keep that behaviour.

use crate::{PywrModel, PywrMultiModel, PywrNetwork};
use schemars::generate::SchemaSettings;
use schemars::transform::RecursiveTransform;
use schemars::{JsonSchema, Schema};
use serde_json::Value;

/// How a node or parameter that is not a valid core definition is treated.
///
/// The deserialisers read a node or parameter as a custom one whenever its core definition fails to
/// deserialise, so a core type with a misspelt or missing field is silently reclassified.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CustomTypes {
    /// Follow the deserialisers: a custom node or parameter may use any `type`, including a core
    /// one, so an invalid core definition is still valid as a custom one.
    Any,
    /// A custom node or parameter must use a `type` that is not a core type (or core alias), so an
    /// invalid core definition is reported.
    NonCoreOnly,
}

/// Generate the root schema for `T` as the deserialisers see it (draft 2020-12).
///
/// Fields that are skipped on serialisation (a name taken from its parent map's key, for example)
/// are still read from a file, so the `writeOnly` annotation `schemars` adds for them is removed.
fn generate<T: JsonSchema>(custom_types: CustomTypes) -> Schema {
    let mut schema = SchemaSettings::draft2020_12()
        .for_deserialize()
        .with_transform(RecursiveTransform(|schema: &mut Schema| {
            schema.remove("writeOnly");
        }))
        .into_generator()
        .into_root_schema_for::<T>();

    if custom_types == CustomTypes::NonCoreOnly {
        exclude_core_types(&mut schema, "CustomNode", CORE_NODE_TYPES);
        exclude_core_types(&mut schema, "CustomParameter", CORE_PARAMETER_TYPES);
        fold_parameter_map_types(&mut schema);
    }
    schema
}

/// The `type` strings the name-keyed `parameters` map reads as `variant`.
///
/// The map's deserialiser lower-cases `type` and strips one trailing `parameter` before it selects a
/// core variant, so a spelling is accepted when its folded form is one of the variant's all
/// lower-case names. A name `x` is therefore reached as `x` or `x` followed by `parameter` (only the
/// outermost suffix is stripped), which is also how a name that itself ends in `parameter` is
/// reached, provided its stem is a name too (`tests::folded_type_pattern_matches_the_deserialiser`
/// checks this). The returned regex is anchored, matches each such name with any casing, and is
/// written for ECMA 262 (JSON Schema) without flags.
fn folded_type_pattern(names: &[&str]) -> String {
    let alternatives: Vec<String> = names
        .iter()
        .copied()
        .filter(|name| *name == name.to_lowercase())
        .map(any_case)
        .collect();
    format!(
        "^(?:{})(?:{})?$",
        alternatives.join("|"),
        any_case(PARAMETER_SUFFIX)
    )
}

const PARAMETER_SUFFIX: &str = "parameter";

/// A regex fragment matching `lower` (all lower case) in any casing under Unicode lower-casing.
///
/// Rust's `str::to_lowercase`, which the deserialiser uses, maps the Kelvin sign U+212A to `k`, so
/// `k` also matches it. No other character in the type names has a non-ASCII upper-case form that
/// lower-cases to it.
fn any_case(lower: &str) -> String {
    lower
        .chars()
        .map(|c| match c {
            'k' => "[kK\\u212A]".to_string(),
            c if c.is_ascii_lowercase() => format!("[{c}{}]", c.to_ascii_uppercase()),
            c => c.to_string(),
        })
        .collect()
}

/// Make the name-keyed `parameters` map check core definitions however `type` is spelt.
///
/// An entry of the map is read as a core parameter when its folded `type` selects a core variant
/// (see [`folded_type_pattern`]), so the core arm accepts each variant under that pattern and the
/// custom arm must avoid every such spelling. Inline parameters (in a node, or inside another
/// parameter) are not folded: their `type` is matched exactly, which the shared `CoreParameter` and
/// `CustomParameter` definitions describe, so they keep those.
fn fold_parameter_map_types(schema: &mut Schema) {
    let mut core = schema
        .pointer("/$defs/CoreParameter")
        .cloned()
        .expect("the schema defines CoreParameter");
    let variants = core
        .get_mut("oneOf")
        .and_then(Value::as_array_mut)
        .expect("a tagged enum schema is a oneOf");
    for (variant, (_, names)) in variants.iter_mut().zip(CORE_PARAMETER_TYPES) {
        let type_schema = variant
            .pointer_mut("/properties/type")
            .and_then(Value::as_object_mut)
            .expect("a tagged variant has a `type` property");
        type_schema.remove("enum");
        type_schema.insert("pattern".to_string(), folded_type_pattern(names).into());
    }

    let every_name: Vec<&str> = CORE_PARAMETER_TYPES
        .iter()
        .flat_map(|(_, names)| names.iter().copied())
        .collect();
    let mut custom = schema
        .pointer("/$defs/CustomParameter")
        .cloned()
        .expect("the schema defines CustomParameter");
    custom
        .pointer_mut("/properties/type")
        .and_then(Value::as_object_mut)
        .expect("a custom definition has a `type` property")
        .insert(
            "not".to_string(),
            serde_json::json!({"pattern": folded_type_pattern(&every_name)}),
        );

    let definitions = schema
        .pointer_mut("/$defs")
        .and_then(Value::as_object_mut)
        .expect("the schema has definitions");
    definitions.insert("CoreParameterAnyCase".to_string(), core);
    definitions.insert("CustomParameterAnyCase".to_string(), custom);
    schema
        .pointer_mut("/$defs/ParameterVec/additionalProperties")
        .expect("ParameterVec describes its entries")
        .clone_from(&serde_json::json!({"anyOf": [
            {"$ref": "#/$defs/CoreParameterAnyCase"},
            {"$ref": "#/$defs/CustomParameterAnyCase"}
        ]}));
}

/// Forbid the definition `custom` from using any `type` that a core definition accepts.
fn exclude_core_types(schema: &mut Schema, custom: &str, types: &[(&str, &[&str])]) {
    let core_types: Vec<&str> = types
        .iter()
        .flat_map(|(tag, aliases)| std::iter::once(tag).chain(aliases.iter()).copied())
        .collect();
    let type_schema = schema
        .pointer_mut(&format!("/$defs/{custom}/properties/type"))
        .and_then(Value::as_object_mut)
        .expect("a custom definition has a `type` property");
    type_schema.insert("not".to_string(), serde_json::json!({"enum": core_types}));
}

/// The JSON Schema for a single-network model file ([`PywrModel`]).
pub fn model_schema(custom_types: CustomTypes) -> Schema {
    generate::<PywrModel>(custom_types)
}

/// The JSON Schema for a multi-model file ([`PywrMultiModel`]).
pub fn multi_model_schema(custom_types: CustomTypes) -> Schema {
    generate::<PywrMultiModel>(custom_types)
}

/// The JSON Schema for a bare network ([`PywrNetwork`]), as read by `pywr-v1-validator --network-only`.
pub fn network_schema(custom_types: CustomTypes) -> Schema {
    generate::<PywrNetwork>(custom_types)
}

// Serde `alias` attributes are not visible to `schemars`, so each tagged variant lists here the
// `type` values the deserialiser accepts for it: the variant name first, then its aliases.
// `tests::aliases_match_serde_attributes` keeps these tables in step with the attributes.
pub(crate) const CORE_NODE_TYPES: &[(&str, &[&str])] = &[
    ("Input", &["input"]),
    ("Link", &["link"]),
    ("Output", &["output"]),
    ("Storage", &["storage"]),
    ("Reservoir", &["reservoir"]),
    ("Catchment", &["catchment"]),
    ("RiverGauge", &["rivergauge", "Rivergauge"]),
    ("LossLink", &["losslink", "Losslink"]),
    ("River", &["river"]),
    ("PiecewiseLink", &["piecewiselink", "Piecewiselink"]),
    ("MultiSplitLink", &["multisplitlink", "Multisplitlink"]),
    ("BreakLink", &["breaklink"]),
    ("Delay", &["delaynode", "DelayNode", "Delaynode"]),
    ("RiverSplit", &["riversplit", "Riversplit"]),
    (
        "RiverSplitWithGauge",
        &["riversplitwithgauge", "Riversplitwithgauge"],
    ),
    ("Aggregated", &["aggregatednode", "AggregatedNode"]),
    (
        "AggregatedStorage",
        &["aggregatedstorage", "Aggregatedstorage"],
    ),
    ("VirtualStorage", &["virtualstorage", "Virtualstorage"]),
    (
        "AnnualVirtualStorage",
        &["annualvirtualstorage", "Annualvirtualstorage"],
    ),
    (
        "MonthlyVirtualStorage",
        &["monthlyvirtualstorage", "Monthlyvirtualstorage"],
    ),
    (
        "SeasonalVirtualStorage",
        &["seasonalvirtualstorage", "Seasonalvirtualstorage"],
    ),
    (
        "RollingVirtualStorage",
        &["rollingvirtualstorage", "Rollingvirtualstorage"],
    ),
];

pub(crate) const CORE_PARAMETER_TYPES: &[(&str, &[&str])] = &[
    (
        "Aggregated",
        &["aggregated", "aggregatedparameter", "AggregatedParameter"],
    ),
    (
        "AggregatedIndex",
        &[
            "aggregatedindex",
            "aggregatedindexparameter",
            "AggregatedIndexParameter",
        ],
    ),
    (
        "AsymmetricSwitchIndex",
        &[
            "asymmetricswitchindex",
            "asymmetricswitchindexparameter",
            "AsymmetricSwitchIndexParameter",
        ],
    ),
    (
        "Constant",
        &["constant", "constantparameter", "ConstantParameter"],
    ),
    (
        "ConstantScenario",
        &[
            "constantscenario",
            "constantscenarioparameter",
            "ConstantScenarioParameter",
        ],
    ),
    (
        "ControlCurvePiecewiseInterpolated",
        &[
            "controlcurvepiecewiseinterpolated",
            "controlcurvepiecewiseinterpolatedparameter",
            "ControlCurvePiecewiseInterpolatedParameter",
        ],
    ),
    (
        "ControlCurveInterpolated",
        &[
            "controlcurveinterpolated",
            "controlcurveinterpolatedparameter",
            "ControlCurveInterpolatedParameter",
        ],
    ),
    (
        "ControlCurveIndex",
        &[
            "controlcurveindex",
            "controlcurveindexparameter",
            "ControlCurveIndexParameter",
        ],
    ),
    (
        "ControlCurve",
        &[
            "controlcurve",
            "controlcurveparameter",
            "ControlCurveParameter",
        ],
    ),
    (
        "DailyProfile",
        &[
            "dailyprofile",
            "dailyprofileparameter",
            "DailyProfileParameter",
        ],
    ),
    (
        "IndexedArray",
        &[
            "indexedarray",
            "indexedarrayparameter",
            "IndexedArrayParameter",
        ],
    ),
    (
        "MonthlyProfile",
        &[
            "monthlyprofile",
            "monthlyprofileparameter",
            "MonthlyProfileParameter",
        ],
    ),
    (
        "WeeklyProfile",
        &[
            "weeklyprofile",
            "weeklyprofileparameter",
            "WeeklyProfileParameter",
        ],
    ),
    (
        "UniformDrawdownProfile",
        &[
            "uniformdrawdownprofile",
            "uniformdrawdownprofileparameter",
            "UniformDrawdownProfileParameter",
        ],
    ),
    ("Max", &["max", "maxparameter", "MaxParameter"]),
    ("Min", &["min", "minparameter", "MinParameter"]),
    (
        "NegativeMin",
        &[
            "negativemin",
            "negativeminparameter",
            "NegativeMinParameter",
        ],
    ),
    (
        "NegativeMax",
        &[
            "negativemax",
            "negativemaxparameter",
            "NegativeMaxParameter",
        ],
    ),
    (
        "Division",
        &["division", "divisionparameter", "DivisionParameter"],
    ),
    (
        "Negative",
        &["negative", "negativeparameter", "NegativeParameter"],
    ),
    (
        "Polynomial1D",
        &[
            "polynomial1d",
            "polynomial1dparameter",
            "Polynomial1DParameter",
        ],
    ),
    (
        "ParameterThreshold",
        &[
            "parameterthreshold",
            "parameterthresholdparameter",
            "ParameterThresholdParameter",
        ],
    ),
    (
        "NodeThreshold",
        &[
            "nodethreshold",
            "nodethresholdparameter",
            "NodeThresholdParameter",
        ],
    ),
    (
        "StorageThreshold",
        &[
            "storagethreshold",
            "storagethresholdparameter",
            "StorageThresholdParameter",
        ],
    ),
    (
        "MultipleThresholdIndex",
        &[
            "multiplethresholdindex",
            "multiplethresholdindexparameter",
            "MultipleThresholdIndexParameter",
        ],
    ),
    (
        "MultipleThresholdParameterIndex",
        &[
            "multiplethresholdparameterindex",
            "multiplethresholdparameterindexparameter",
            "MultipleThresholdparameterIndexParameter",
        ],
    ),
    (
        "CurrentYearThreshold",
        &[
            "currentyearthreshold",
            "currentyearthresholdparameter",
            "CurrentYearThresholdParameter",
        ],
    ),
    (
        "CurrentOrdinalDayThreshold",
        &[
            "currentordinaldaythreshold",
            "currentordinaldaythresholdparameter",
            "CurrentOrdinalDayThresholdParameter",
        ],
    ),
    (
        "TablesArray",
        &[
            "tablesarray",
            "tablesarrayparameter",
            "TablesArrayParameter",
        ],
    ),
    (
        "DataFrame",
        &["dataframe", "dataframeparameter", "DataFrameParameter"],
    ),
    (
        "Deficit",
        &["deficit", "deficitparameter", "DeficitParameter"],
    ),
    (
        "DiscountFactor",
        &[
            "discountfactor",
            "discountfactorparameter",
            "DiscountFactorParameter",
        ],
    ),
    (
        "InterpolatedVolume",
        &[
            "interpolatedvolume",
            "interpolatedvolumeparameter",
            "InterpolatedVolumeParameter",
        ],
    ),
    (
        "InterpolatedFlow",
        &[
            "interpolatedflow",
            "interpolatedflowparameter",
            "InterpolatedFlowParameter",
        ],
    ),
    (
        "HydropowerTarget",
        &[
            "hydropowertarget",
            "hydropowertargetparameter",
            "HydropowerTargetParameter",
        ],
    ),
    (
        "Storage",
        &["storage", "storageparameter", "StorageParameter"],
    ),
    (
        "RollingMeanFlowNode",
        &[
            "rollingmeanflownode",
            "rollingmeanflownodeparameter",
            "RollingMeanFlowNodeParameter",
        ],
    ),
    (
        "ScenarioWrapper",
        &[
            "scenariowrapper",
            "scenariowrapperparameter",
            "ScenarioWrapperParameter",
        ],
    ),
    ("Flow", &["flow", "flowparameter", "FlowParameter"]),
    (
        "RbfProfile",
        &["rbfprofile", "rbfprofileparameter", "RbfProfileParameter"],
    ),
];

/// Replace the `type` constant of each variant of a tagged enum schema with the full list of values
/// the deserialiser accepts for it.
fn add_type_aliases(schema: &mut Schema, types: &[(&str, &[&str])]) {
    let variants = schema
        .get_mut("oneOf")
        .and_then(Value::as_array_mut)
        .expect("a tagged enum schema is a oneOf");
    assert_eq!(variants.len(), types.len(), "one oneOf entry per variant");
    for (variant, (tag, aliases)) in variants.iter_mut().zip(types) {
        let type_schema = variant
            .pointer_mut("/properties/type")
            .and_then(Value::as_object_mut)
            .expect("a tagged variant has a `type` property");
        assert_eq!(type_schema.get("const"), Some(&Value::from(*tag)));
        type_schema.remove("const");
        let accepted: Vec<&str> = std::iter::once(tag)
            .chain(aliases.iter())
            .copied()
            .collect();
        type_schema.insert("enum".to_string(), accepted.into());
    }
}

/// Let a struct schema accept `alias` as another name for the property `field`, as a serde
/// `alias` attribute does (`schemars` does not read field aliases). A required field becomes a
/// choice between its two names; serde rejects an object that supplies both.
fn accept_field_alias(schema: &mut Schema, field: &str, alias: &str) {
    let object = schema.ensure_object();
    let properties = object
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("a struct schema has properties");
    let property = properties
        .get(field)
        .cloned()
        .expect("the aliased field is a property");
    properties.insert(alias.to_string(), property);

    let required = object.get_mut("required").and_then(Value::as_array_mut);
    if let Some(required) = required
        && let Some(position) = required.iter().position(|name| name == field)
    {
        required.remove(position);
        if required.is_empty() {
            object.remove("required");
        }
        object.insert(
            "oneOf".to_string(),
            serde_json::json!([{"required": [field]}, {"required": [alias]}]),
        );
    }
}

/// `IndexedArrayParameter::parameters` is also read from `params`.
pub(crate) fn indexed_array_transform(schema: &mut Schema) {
    accept_field_alias(schema, "parameters", "params");
}

/// `ConstantParameter::value` is also read from `values`.
pub(crate) fn constant_parameter_transform(schema: &mut Schema) {
    accept_field_alias(schema, "value", "values");
}

/// `Predicate` variants are also read as lower-case names and as comparison symbols.
pub(crate) fn predicate_transform(schema: &mut Schema) {
    *schema = schemars::json_schema!({
        "type": "string",
        "enum": [
            "LT", "<", "lt", "GT", ">", "gt", "EQ", "==", "eq", "LE", "<=", "le", "GE", ">=", "ge"
        ]
    });
}

pub(crate) fn core_node_transform(schema: &mut Schema) {
    add_type_aliases(schema, CORE_NODE_TYPES);
}

pub(crate) fn core_parameter_transform(schema: &mut Schema) {
    add_type_aliases(schema, CORE_PARAMETER_TYPES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::CoreNode;
    use crate::parameters::CoreParameter;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    /// The `alias = "..."` literals of a source file, sorted.
    fn alias_literals(source: &str) -> Vec<String> {
        let mut aliases: Vec<String> = source
            .split("alias = \"")
            .skip(1)
            .map(|rest| rest.split('"').next().expect("closing quote").to_string())
            .collect();
        aliases.sort();
        aliases
    }

    fn table_aliases(table: &[(&str, &[&str])]) -> Vec<String> {
        let mut aliases: Vec<String> = table
            .iter()
            .flat_map(|(_, aliases)| aliases.iter().map(|a| a.to_string()))
            .collect();
        aliases.sort();
        aliases
    }

    #[test]
    fn aliases_match_serde_attributes() {
        assert_eq!(
            alias_literals(include_str!("nodes/mod.rs")),
            table_aliases(CORE_NODE_TYPES)
        );
        assert_eq!(
            alias_literals(include_str!("parameters/mod.rs")),
            table_aliases(CORE_PARAMETER_TYPES)
        );
    }

    /// Every type the schema lists for a variant must select a variant when deserialised; an
    /// unselected tag fails with "unknown variant", whatever else is missing from the object.
    fn assert_types_select_a_variant<T: DeserializeOwned>(table: &[(&str, &[&str])]) {
        for (tag, aliases) in table {
            for accepted in std::iter::once(tag).chain(aliases.iter()) {
                let result = serde_json::from_value::<T>(json!({"type": accepted, "name": "n"}));
                if let Err(e) = result {
                    assert!(
                        !e.to_string().contains("unknown variant"),
                        "type {accepted:?} is listed for {tag} but not accepted: {e}"
                    );
                }
            }
        }
    }

    #[test]
    fn listed_types_are_accepted_by_the_deserialisers() {
        assert_types_select_a_variant::<CoreNode>(CORE_NODE_TYPES);
        assert_types_select_a_variant::<CoreParameter>(CORE_PARAMETER_TYPES);
    }

    /// A validator for strings that match `pattern` (a JSON Schema `pattern`).
    fn pattern_validator(pattern: &str) -> jsonschema::Validator {
        jsonschema::validator_for(&json!({"type": "string", "pattern": pattern}))
            .expect("the pattern is valid")
    }

    /// How the name-keyed `parameters` map folds `type` before it selects a core variant.
    fn fold(ty: &str) -> String {
        let lower = ty.to_lowercase();
        lower
            .strip_suffix(PARAMETER_SUFFIX)
            .unwrap_or(&lower)
            .to_string()
    }

    fn spellings(name: &str) -> Vec<String> {
        let mut alternating = String::new();
        for (i, c) in name.chars().enumerate() {
            alternating.extend(if i % 2 == 0 {
                c.to_uppercase().collect::<Vec<_>>()
            } else {
                vec![c]
            });
        }
        let mut titled = name.to_string();
        titled[..1].make_ascii_uppercase();
        vec![
            name.to_string(),
            name.to_uppercase(),
            titled.clone(),
            alternating,
            format!("{name}parameter"),
            format!("{name}Parameter"),
            format!("{}PARAMETER", name.to_uppercase()),
            format!("{titled}Parameter"),
            format!("{name}parameterparameter"),
            format!("{name}x"),
            format!("x{name}"),
            format!("{name} "),
            name[..name.len() - 1].to_string(),
            name.replace('k', "\u{212A}"),
        ]
    }

    /// The pattern the strict schema gives the name-keyed parameters map must select exactly the
    /// spellings that the deserialiser folds onto a core variant, and each variant's own pattern
    /// those that fold onto that variant.
    #[test]
    fn folded_type_pattern_matches_the_deserialiser() {
        let every_name: Vec<&str> = CORE_PARAMETER_TYPES
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect();
        let all = pattern_validator(&folded_type_pattern(&every_name));
        for (tag, names) in CORE_PARAMETER_TYPES {
            let own = pattern_validator(&folded_type_pattern(names));
            for name in names.iter().filter(|n| **n == n.to_lowercase()) {
                for spelling in spellings(name) {
                    let folded = fold(&spelling);
                    let selected = serde_json::from_value::<CoreParameter>(
                        json!({"type": folded, "name": "n"}),
                    );
                    let accepted = selected
                        .as_ref()
                        .err()
                        .is_none_or(|e| !e.to_string().contains("unknown variant"));
                    assert_eq!(
                        all.is_valid(&json!(spelling)),
                        accepted,
                        "{spelling:?} for {tag}"
                    );
                    if names.contains(&folded.as_str()) {
                        assert!(own.is_valid(&json!(spelling)), "{spelling:?} for {tag}");
                    }
                }
            }
        }
    }

    /// The map really reads each folded spelling as a core parameter, so the pattern above
    /// describes the deserialiser and not a copy of its fold.
    #[test]
    fn parameter_map_reads_any_casing_as_core() {
        for ty in [
            "CONSTANT",
            "constantParameter",
            "Constant",
            "constantparameterparameter",
        ] {
            let parameters: crate::parameters::ParameterVec =
                serde_json::from_value(json!({"p": {"type": ty, "value": 1.0}}))
                    .expect("the map deserialises");
            assert!(parameters.iter().all(|p| !p.is_custom()), "{ty}");
        }
    }
}
