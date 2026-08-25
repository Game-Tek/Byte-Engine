fn main() -> Result<(), i32> {
	let _ = simple_logger::SimpleLogger::new().env().init();

	let color = parse_color_choice(std::env::args());
	let matches = CLI::command().color(color).get_matches();

	let cli = CLI::from_arg_matches(&matches).map_err(|error| {
		let _ = error.print();
		2
	})?;

	let executor = resource_management::r#async::Executor::new().map_err(|error| {
		log::error!(
			"Failed to start BELD asynchronous resource access. The most likely cause is that the platform I/O driver could not be initialized. Error: {error}"
		);
		1
	})?;

	executor.block_on(run(cli))
}

/// Dispatches one parsed command through BELD's asynchronous library API.
async fn run(cli: CLI) -> Result<(), i32> {
	let source_path = cli.source;
	let destination_path = cli.destination;
	let storage_mode = cli.storage_mode.map(Into::into);
	let _color = cli.color;

	match cli.command {
		Commands::Wipe {} => beld::wipe(destination_path).await,
		Commands::Clear {} => beld::clear(destination_path).await,
		Commands::List {} => beld::list(destination_path).await,
		Commands::Query {
			class,
			properties,
			limit,
			cursor,
			format,
		} => beld::query(destination_path, class, properties, limit, cursor, format).await,
		Commands::Inspect { id, format } => beld::inspect(destination_path, id, format).await,
		Commands::Bake {
			ids,
			memory_budget,
			texture_compression,
		} => {
			beld::bake(
				source_path,
				destination_path,
				ids,
				storage_mode,
				texture_compression.map(Into::into),
				bake_memory_budget(memory_budget),
			)
			.await
		}
		Commands::Delete { ids } => beld::delete(destination_path, ids).await,
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
struct CLI {
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
		#[arg(long, value_enum, default_value_t = OutputFormat::Human)]
		format: OutputFormat,
	},
	/// Inspect one resource.
	Inspect {
		/// The resource ID or UID to inspect.
		/// Example: `beld inspect mesh.gltf#image` or `beld inspect d41d8cd98f00b204e9800998ecf8427e`
		id: String,
		#[arg(long, value_enum, default_value_t = OutputFormat::Human)]
		format: OutputFormat,
	},
	/// Bake source assets into resources.
	Bake {
		/// The soft memory budget for concurrent bake arenas, in MiB.
		/// By default, BELD uses half of the system memory available when the command starts.
		#[arg(long = "memory-budget-mib", value_parser = parse_memory_budget_mib)]
		memory_budget: Option<NonZeroUsize>,
		/// Native transport compression for baked texture files.
		#[arg(long, value_enum)]
		texture_compression: Option<TextureCompression>,
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

#[derive(Clone, Copy, ValueEnum)]
enum TextureCompression {
	None,
	MetalIoLz4,
}

impl From<TextureCompression> for resource_management::resource::ResourceCompression {
	fn from(value: TextureCompression) -> Self {
		match value {
			TextureCompression::None => Self::None,
			TextureCompression::MetalIoLz4 => Self::MetalIoLz4,
		}
	}
}

impl From<StorageMode> for resource_management::resource::ResourceStorageMode {
	fn from(value: StorageMode) -> Self {
		match value {
			StorageMode::Files => Self::Files,
			StorageMode::Packed => Self::Packed,
		}
	}
}

use std::num::NonZeroUsize;

use beld::OutputFormat;
use clap::{
	CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum,
	builder::styling::{AnsiColor, Effects, Styles},
};
