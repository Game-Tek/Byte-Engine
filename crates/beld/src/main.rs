use clap::{
	builder::styling::{AnsiColor, Effects, Styles},
	CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum,
};

mod commands;
mod utils;

const CLAP_STYLING: Styles = Styles::styled()
	.header(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
	.usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
	.literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
	.placeholder(AnsiColor::Cyan.on_default())
	.error(AnsiColor::Red.on_default().effects(Effects::BOLD))
	.valid(AnsiColor::Green.on_default().effects(Effects::BOLD))
	.invalid(AnsiColor::Red.on_default().effects(Effects::BOLD));

#[derive(Parser)]
#[command(version, about, long_about = None, color = clap::ColorChoice::Auto, styles = CLAP_STYLING)]
struct Cli {
	/// The full path to the assets directory.
	/// Example: `beld --source assets`
	#[arg(short, long, default_value = "assets")]
	source: String,

	/// The full path to the resources directory.
	/// Example: `beld --destination resources`
	#[arg(short, long, default_value = "resources")]
	destination: String,

	/// When to use terminal colors.
	#[arg(long, global = true, value_enum, default_value_t = clap::ColorChoice::Auto)]
	color: clap::ColorChoice,

	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand)]
enum Commands {
	/// Wipe all resources, same as clear
	Wipe {},
	/// Clear all resources, same as wipe
	Clear {},
	/// List all resources
	List {},
	/// Query resources by class and available indexed properties
	Query {
		/// The resource class to query.
		/// Example: `beld query Material group=opaque tag=hero`
		class: String,
		/// Property equality filters in `property=value` form.
		/// Example: `beld query Material name=materials/hero`
		#[clap(value_delimiter = ' ', num_args = 0..)]
		properties: Vec<String>,
		/// The maximum amount of resources to return.
		#[arg(long)]
		limit: Option<usize>,
		/// The cursor printed by a previous query page.
		#[arg(long)]
		cursor: Option<String>,
		#[arg(long, value_enum, default_value_t = QueryFormat::Human)]
		format: QueryFormat,
	},
	/// Inspect a resource
	Inspect {
		/// The ID or UID of the resource to inspect.
		/// Example: `beld inspect mesh.gltf#image` or `beld inspect d41d8cd98f00b204e9800998ecf8427e`
		id: String,
		#[arg(long, value_enum, default_value_t = InspectFormat::Human)]
		format: InspectFormat,
	},
	/// Bake assets into resources
	Bake {
		/// The IDs of the resources to bake.
		/// Example: `beld bake audio.wav mesh.gltf mesh.gltf#image`
		#[clap(value_delimiter = ' ', num_args = 1..)]
		ids: Vec<String>,
	},
	/// Delete resources
	Delete {
		/// The IDs of the resources to delete.
		/// Example: `beld delete audio.wav mesh.gltf mesh.gltf#image`
		#[clap(value_delimiter = ' ', num_args = 1..)]
		ids: Vec<String>,
	},
}

#[derive(Clone, Copy, ValueEnum)]
pub enum InspectFormat {
	Human,
	Json,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum QueryFormat {
	Human,
	Json,
}

fn main() -> Result<(), i32> {
	let _ = simple_logger::SimpleLogger::new().env().init();

	let color = parse_color_choice(std::env::args());
	let matches = Cli::command().color(color).get_matches();
	let cli = Cli::from_arg_matches(&matches).map_err(|error| {
		let _ = error.print();
		2
	})?;

	let command = cli.command;

	let source_path = cli.source;
	let destination_path = cli.destination;
	let _color = cli.color;

	match command {
		Commands::Wipe {} => commands::wipe(destination_path),
		Commands::Clear {} => commands::wipe(destination_path),
		Commands::List {} => commands::list(destination_path),
		Commands::Query {
			class,
			properties,
			limit,
			cursor,
			format,
		} => commands::query(destination_path, class, properties, limit, cursor, format),
		Commands::Inspect { id, format } => commands::inspect(destination_path, id, format),
		Commands::Bake { ids } => commands::bake(source_path, destination_path, ids),
		Commands::Delete { ids } => commands::delete(destination_path, ids),
	}
}

/// Pre-scans CLI arguments so help and parser errors can honor `--color` before Clap fully parses the command.
fn parse_color_choice(args: impl IntoIterator<Item = String>) -> clap::ColorChoice {
	let mut args = args.into_iter();
	while let Some(arg) = args.next() {
		let value = if arg == "--color" {
			args.next()
		} else {
			arg.strip_prefix("--color=").map(str::to_string)
		};

		match value.as_deref() {
			Some("always") => return clap::ColorChoice::Always,
			Some("never") => return clap::ColorChoice::Never,
			Some("auto") => return clap::ColorChoice::Auto,
			_ => {}
		}
	}

	clap::ColorChoice::Auto
}

#[cfg(test)]
mod tests {
	use clap::Parser as _;

	use super::{parse_color_choice, Cli, Commands, InspectFormat, QueryFormat};

	fn args(values: &[&str]) -> Vec<String> {
		values.iter().map(|value| (*value).to_string()).collect()
	}

	#[test]
	fn color_pre_scan_supports_split_and_equals_forms() {
		assert_eq!(
			parse_color_choice(args(&["beld", "--color", "always", "list"])),
			clap::ColorChoice::Always
		);
		assert_eq!(
			parse_color_choice(args(&["beld", "list", "--color=never"])),
			clap::ColorChoice::Never
		);
		assert_eq!(
			parse_color_choice(args(&["beld", "--color=auto", "list"])),
			clap::ColorChoice::Auto
		);
	}

	#[test]
	fn color_pre_scan_ignores_missing_and_invalid_values() {
		assert_eq!(parse_color_choice(args(&["beld", "--color"])), clap::ColorChoice::Auto);
		assert_eq!(
			parse_color_choice(args(&["beld", "--color=rainbow", "list"])),
			clap::ColorChoice::Auto
		);
		assert_eq!(parse_color_choice(args(&["beld", "list"])), clap::ColorChoice::Auto);
	}

	#[test]
	fn cli_defaults_paths_and_parses_query_contract() {
		let cli = Cli::try_parse_from([
			"beld",
			"query",
			"Material",
			"name=hero",
			"group=opaque",
			"--limit",
			"25",
			"--format",
			"json",
		])
		.unwrap();

		assert_eq!(cli.source, "assets");
		assert_eq!(cli.destination, "resources");
		match cli.command {
			Commands::Query {
				class,
				properties,
				limit,
				cursor,
				format,
			} => {
				assert_eq!(class, "Material");
				assert_eq!(properties, ["name=hero", "group=opaque"]);
				assert_eq!(limit, Some(25));
				assert_eq!(cursor, None);
				assert!(matches!(format, QueryFormat::Json));
			}
			_ => panic!("Expected query command. The most likely cause is a CLI subcommand parsing regression."),
		}
	}

	#[test]
	fn cli_honors_global_paths_and_inspect_format() {
		let cli = Cli::try_parse_from([
			"beld",
			"--source",
			"input",
			"--destination",
			"output",
			"inspect",
			"mesh#0",
			"--format",
			"json",
		])
		.unwrap();

		assert_eq!(cli.source, "input");
		assert_eq!(cli.destination, "output");
		assert!(matches!(
			cli.command,
			Commands::Inspect {
				id,
				format: InspectFormat::Json
			} if id == "mesh#0"
		));
	}
}
