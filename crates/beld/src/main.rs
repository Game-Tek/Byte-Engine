/// Exits with the code a command returns, so invalid arguments the library detects also exit with 2.
fn main() -> ExitCode {
	match run_cli() {
		Ok(()) => ExitCode::SUCCESS,
		Err(code) => ExitCode::from(code),
	}
}

fn run_cli() -> Result<(), u8> {
	let _ = simple_logger::SimpleLogger::new().env().init();

	let color = parse_color_choice(std::env::args());
	let matches = Cli::command().color(color).get_matches();

	let cli = Cli::from_arg_matches(&matches).map_err(|error| {
		let _ = error.print();
		2u8
	})?;

	let executor = resource_management::r#async::Executor::new().map_err(|error| {
		log::error!(
			"Failed to start BELD asynchronous resource access. The most likely cause is that the platform I/O driver could not be initialized. Error: {error}"
		);
		1u8
	})?;

	executor.block_on(run(cli)).map_err(|error| {
		log::error!("{error}");
		error.exit_code()
	})
}

/// Dispatches one parsed command through BELD's asynchronous library API.
async fn run(cli: Cli) -> Result<(), BeldError> {
	let source = Path::new(&cli.source);
	let destination = Path::new(&cli.destination);
	match cli.command {
		Commands::Wipe {} => beld::wipe(destination).await,
		Commands::Clear {} => beld::clear(destination).await,
		Commands::List {} => beld::list(destination).await,
		Commands::Query {
			class,
			properties,
			limit,
			cursor,
			format,
		} => beld::query(destination, class, properties, limit, cursor, format).await,
		Commands::Inspect { id, format, payload } => beld::inspect(destination, id, format, payload).await,
		Commands::Bake {
			ids,
			memory_budget,
			texture_compression,
			force,
			config,
			dry_run,
			format,
			payload,
		} => {
			let configuration = read_configuration_file(Path::new(&config))
				.map_err(|error| BeldError::new(format!("Failed to read the project configuration. {error}")))?;
			let features = VisibilityFeatures::from_parameters(configuration.as_slice().scope("render"));
			if dry_run {
				return beld::bake_dry_run(
					source,
					ids,
					bake_memory_budget(memory_budget),
					features,
					format.unwrap_or(OutputFormat::Human),
					payload.unwrap_or_default(),
					&mut std::io::stdout().lock(),
				)
				.await;
			}
			beld::bake(
				source,
				destination,
				ids,
				cli.storage_mode.map(Into::into),
				texture_compression.map(Into::into),
				bake_memory_budget(memory_budget),
				force,
				features,
			)
			.await
		}
		Commands::Delete { ids } => beld::delete(destination, ids).await,
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

/// Returns the configured bake budget, or BELD's default budget for this system.
fn bake_memory_budget(configured: Option<NonZeroUsize>) -> NonZeroUsize {
	configured.unwrap_or_else(beld::pipeline::default_bake_memory_budget)
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

		if let Some(choice) = value.and_then(|value| clap::ColorChoice::from_str(&value, false).ok()) {
			return choice;
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
		/// Adds each payload stream's bytes, decoded into values or as base64. Needs `--format json`.
		/// Example: `beld inspect mesh.gltf --format json --payload typed`
		#[arg(long, value_enum, default_value_t = Payload::None)]
		payload: Payload,
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
		/// Bake into memory and print every resource the bake produces, without reading or writing the destination.
		/// Requires asset IDs.
		/// Example: `beld bake --dry-run --format json --payload typed mesh.gltf`
		#[arg(long, requires = "ids", conflicts_with_all = ["force", "texture_compression"])]
		dry_run: bool,
		/// The dry-run output format. Defaults to `human`.
		#[arg(long, value_enum, requires = "dry_run")]
		format: Option<OutputFormat>,
		/// Adds each payload stream's bytes to the dry-run output, decoded into values or as base64. Needs
		/// `--format json`.
		#[arg(long, value_enum, requires = "dry_run")]
		payload: Option<Payload>,
		/// Rebake every selected asset, even when its resource is current with its source files.
		/// Use it after changing an asset processor, `--texture-compression`, or a baked setting in `--config`, which
		/// source versions don't track.
		/// Example: `beld bake --force`
		#[arg(long)]
		force: bool,
		/// The project's configuration file, which the application also reads at startup.
		/// Its `render.gtao.enabled` decides whether GTAO is baked into material shaders.
		/// Example: `beld bake --config config.json`
		#[arg(long, default_value = "config.json")]
		config: String,
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

impl From<TextureCompression> for resource_management::resource::ResourceGpuCompressionPolicy {
	fn from(value: TextureCompression) -> Self {
		match value {
			TextureCompression::None => Self::Disabled,
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

use std::{num::NonZeroUsize, path::Path, process::ExitCode};

use beld::{BeldError, OutputFormat, Payload};
use byte_engine::application::parameters::{Parameters as _, read_configuration_file};
use byte_engine::rendering::pipelines::visibility::VisibilityFeatures;
use clap::{
	CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum,
	builder::styling::{AnsiColor, Effects, Styles},
};
