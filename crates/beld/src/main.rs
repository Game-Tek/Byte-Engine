use std::num::NonZeroUsize;

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
	/// The path to the source assets directory.
	/// Example: `beld --source assets`
	#[arg(short, long, default_value = "assets")]
	source: String,

	/// The path to the baked resources directory.
	/// Example: `beld --destination resources`
	#[arg(short, long, default_value = "resources")]
	destination: String,

	/// When to use terminal colors.
	#[arg(long, global = true, value_enum, default_value_t = clap::ColorChoice::Auto)]
	color: clap::ColorChoice,

	/// How a new resource store persists binary payloads.
	/// Existing stores keep their persisted mode.
	#[arg(long, global = true, value_enum)]
	storage_mode: Option<StorageMode>,

	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand)]
enum Commands {
	/// Remove all baked resources. This command is the same as `clear`.
	Wipe {},
	/// Remove all baked resources. This command is the same as `wipe`.
	Clear {},
	/// List all baked resources.
	List {},
	/// Find resources by class and indexed property values.
	Query {
		/// The resource class to query.
		/// Example: `beld query Material group=opaque tag=hero`
		class: String,
		/// Property equality filters in `property=value` form.
		/// Example: `beld query Material name=materials/hero`
		#[clap(value_delimiter = ' ', num_args = 0..)]
		properties: Vec<String>,
		/// The maximum number of resources to return.
		#[arg(long)]
		limit: Option<usize>,
		/// The cursor printed by a previous query page.
		#[arg(long)]
		cursor: Option<String>,
		#[arg(long, value_enum, default_value_t = QueryFormat::Human)]
		format: QueryFormat,
	},
	/// Inspect one resource.
	Inspect {
		/// The resource ID or UID to inspect.
		/// Example: `beld inspect mesh.gltf#image` or `beld inspect d41d8cd98f00b204e9800998ecf8427e`
		id: String,
		#[arg(long, value_enum, default_value_t = InspectFormat::Human)]
		format: InspectFormat,
	},
	/// Bake source assets into resources.
	Bake {
		/// The soft memory budget for concurrent bake arenas, in MiB.
		/// By default, BELD uses half of the system memory available when the command starts.
		#[arg(long = "memory-budget-mib", value_parser = parse_memory_budget_mib)]
		memory_budget: Option<NonZeroUsize>,
		/// The asset IDs to bake. If omitted, BELD recursively bakes all supported assets under the source directory.
		/// Example: `beld bake audio.wav mesh.gltf mesh.gltf#image`
		#[clap(value_delimiter = ' ', num_args = 0..)]
		ids: Vec<String>,
	},
	/// Delete specific resources.
	Delete {
		/// The IDs of the resources to delete.
		/// Example: `beld delete audio.wav mesh.gltf mesh.gltf#image`
		#[clap(value_delimiter = ' ', num_args = 1..)]
		ids: Vec<String>,
	},
}

#[derive(Clone, Copy, ValueEnum)]
enum StorageMode {
	Files,
	Packed,
}

impl From<StorageMode> for resource_management::resource::ResourceStorageMode {
	fn from(value: StorageMode) -> Self {
		match value {
			StorageMode::Files => Self::Files,
			StorageMode::Packed => Self::Packed,
		}
	}
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
	let storage_mode = cli.storage_mode.map(Into::into);
	let _color = cli.color;

	let executor = resource_management::r#async::Executor::new().map_err(|error| {
		log::error!(
			"Failed to start BELD asynchronous resource access. The most likely cause is that the platform I/O driver could not be initialized. Error: {error}"
		);
		1
	})?;

	match command {
		Commands::Wipe {} => commands::wipe(destination_path),
		Commands::Clear {} => commands::wipe(destination_path),
		Commands::List {} => executor.block_on(commands::list(destination_path)),
		Commands::Query {
			class,
			properties,
			limit,
			cursor,
			format,
		} => executor.block_on(commands::query(destination_path, class, properties, limit, cursor, format)),
		Commands::Inspect { id, format } => executor.block_on(commands::inspect(destination_path, id, format)),
		Commands::Bake { ids, memory_budget } => commands::bake(
			source_path,
			destination_path,
			ids,
			storage_mode,
			bake_memory_budget(memory_budget),
		),
		Commands::Delete { ids } => commands::delete(destination_path, ids),
	}
}

/// Parses a positive MiB count and converts it to bytes without silently saturating.
fn parse_memory_budget_mib(value: &str) -> Result<NonZeroUsize, String> {
	let invalid =
		|| "Invalid memory budget. The value must be a positive MiB count that fits this platform's address size.".to_string();
	let mib = value.parse::<usize>().ok().and_then(NonZeroUsize::new).ok_or_else(invalid)?;
	mib.get()
		.checked_mul(1024 * 1024)
		.and_then(NonZeroUsize::new)
		.ok_or_else(invalid)
}

/// Returns the configured bake budget or preserves half of currently available system memory as headroom.
fn bake_memory_budget(configured: Option<NonZeroUsize>) -> NonZeroUsize {
	if let Some(configured) = configured {
		return configured;
	}

	let mut system = sysinfo::System::new();
	system.refresh_memory();
	let available_bytes = usize::try_from(system.available_memory()).unwrap_or(usize::MAX);
	NonZeroUsize::new(available_bytes / 2).unwrap_or(NonZeroUsize::MIN)
}

/// Reads `--color` before the full parse so help and parser errors use the selected color mode.
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

	use super::{parse_color_choice, Cli, Commands, InspectFormat, QueryFormat, StorageMode};

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
	fn cli_bake_allows_no_ids_and_selects_payload_storage() {
		let cli = Cli::try_parse_from(["beld", "bake"]).unwrap();

		assert!(cli.storage_mode.is_none());
		assert!(matches!(
			cli.command,
			Commands::Bake {
				ids,
				memory_budget: None
			} if ids.is_empty()
		));

		let cli = Cli::try_parse_from([
			"beld",
			"--storage-mode",
			"packed",
			"bake",
			"--memory-budget-mib",
			"1536",
			"mesh.gltf",
			"mesh.gltf#skeleton",
		])
		.unwrap();

		assert!(matches!(cli.storage_mode, Some(StorageMode::Packed)));
		assert!(matches!(
			cli.command,
			Commands::Bake {
				ids,
				memory_budget: Some(memory_budget)
			} if ids == ["mesh.gltf", "mesh.gltf#skeleton"] && memory_budget.get() == 1536 * 1024 * 1024
		));
	}

	#[test]
	fn cli_bake_rejects_zero_and_overflowing_memory_budgets() {

		assert!(Cli::try_parse_from(["beld", "bake", "--memory-budget-mib", "0"]).is_err());
		assert!(Cli::try_parse_from([
			"beld".to_string(),
			"bake".to_string(),
			"--memory-budget-mib".to_string(),
			usize::MAX.to_string(),
		])
		.is_err());
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
