use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;

/// Which Pywr v1 document the exported JSON Schema describes.
#[derive(Copy, Clone, Debug, ValueEnum)]
enum SchemaKind {
    /// A single-network model file.
    Model,
    /// A multi-model file.
    MultiModel,
    /// A bare network, as read with `--network-only`.
    Network,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write the JSON Schema for the Pywr v1 model format.
    ExportSchema {
        /// Path to save the JSON schema.
        out: std::path::PathBuf,
        /// The kind of document to describe.
        #[arg(short, long, value_enum, default_value_t = SchemaKind::Model)]
        kind: SchemaKind,
        /// Report invalid core nodes and parameters instead of accepting them as custom ones.
        ///
        /// The deserialisers read a node or parameter as a custom one whenever its core definition
        /// fails to parse. With this flag a custom node or parameter must use a type that is not a
        /// core type, so the error in a core definition is reported.
        #[arg(short, long)]
        strict: bool,
    },
}

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, args_conflicts_with_subcommands = true)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(short, long, required = true)]
    path: Option<std::path::PathBuf>,
    #[arg(short, long)]
    network_only: bool,
}

fn export_schema(out: &std::path::Path, kind: SchemaKind, strict: bool) {
    use pywr_v1_schema::json_schema::{
        CustomTypes, model_schema, multi_model_schema, network_schema,
    };

    let custom_types = if strict {
        CustomTypes::NonCoreOnly
    } else {
        CustomTypes::Any
    };
    let schema = match kind {
        SchemaKind::Model => model_schema(custom_types),
        SchemaKind::MultiModel => multi_model_schema(custom_types),
        SchemaKind::Network => network_schema(custom_types),
    };
    let json = serde_json::to_string_pretty(&schema).expect("Failed to serialise the schema.");
    std::fs::write(out, json).expect("Failed to write the schema file.");
}

fn main() {
    let args = Args::parse();

    if let Some(Command::ExportSchema { out, kind, strict }) = args.command {
        export_schema(&out, kind, strict);
        return;
    }

    let path = args
        .path
        .expect("clap requires --path without a subcommand");

    println!("Path: {:?}", path);

    let file = File::open(path).expect("Could not open file.");
    let reader = BufReader::new(file);

    let network: pywr_v1_schema::PywrNetwork = if args.network_only {
        serde_json::from_reader(reader).expect("Failed to parse Pywr JSON file.")
    } else {
        let model: pywr_v1_schema::PywrModel =
            serde_json::from_reader(reader).expect("Failed to parse Pywr JSON file.");

        model.network
    };

    println!("Parsed Pywr JSON file successfully!");

    {
        // Identify custom nodes
        let custom_types: HashSet<_> = match network.nodes {
            Some(nodes) => nodes
                .iter()
                .filter_map(|n| {
                    if let pywr_v1_schema::nodes::Node::Custom(c) = n {
                        Some(c.ty.clone())
                    } else {
                        None
                    }
                })
                .collect(),
            None => HashSet::new(),
        };

        let mut custom_types: Vec<_> = custom_types.into_iter().collect();
        custom_types.sort();

        if !custom_types.is_empty() {
            println!("Found {} custom node types:", custom_types.len());
            for ty in custom_types {
                println!("  {}", ty);
            }
        } else {
            println!("No custom nodes found!")
        }
    }

    if let Some(parameters) = network.parameters {
        let custom_types: HashSet<_> = parameters
            .iter()
            .filter_map(|p| {
                if let pywr_v1_schema::parameters::Parameter::Custom(c) = p {
                    Some(c.ty.clone())
                } else {
                    None
                }
            })
            .collect();

        let mut custom_types: Vec<_> = custom_types.into_iter().collect();
        custom_types.sort();

        if !custom_types.is_empty() {
            println!("Found {} custom parameter types:", custom_types.len());
            for ty in custom_types {
                println!("  {}", ty);
            }
        } else {
            println!("No custom parameters found!")
        }
    }
}
