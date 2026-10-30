//! The BELD command line, which every application's BELD binary runs with its own baking setup.

/// Runs the BELD command line for an application, and returns the exit code of the command it ran.
///
/// Call it from the `main` of the application's BELD binary. `parameters` are the parameters the application starts
/// with, before its `config.json` and `BE_*` variables override them, and `register` is the function its debug runtime
/// passes to [`setup_default_resource_and_asset_management`]. With both, a bake produces the resources a debug run
/// does. An application without parameters or formats of its own passes `&[]` and
/// [`register_default_asset_handlers`]. Invalid arguments exit with 2, and a failed command with 1.
///
/// [`setup_default_resource_and_asset_management`]: byte_engine::application::graphics::setup_default_resource_and_asset_management
/// [`register_default_asset_handlers`]: byte_engine::application::graphics::register_default_asset_handlers
pub fn run(parameters: &[Parameter], register: impl FnOnce(&mut AssetManager, &[Parameter])) -> ExitCode {
	match run_cli(parameters, register) {
		Ok(()) => ExitCode::SUCCESS,
		Err(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
	}
}

/// Parses the arguments, then runs the command they select.
fn run_cli(parameters: &[Parameter], register: impl FnOnce(&mut AssetManager, &[Parameter])) -> Result<(), i32> {
	let color = parse_color_choice(std::env::args());
	// BELD's own output reads the choice from here, separately for each stream.
	anstream::ColorChoice::write_global(match color {
		clap::ColorChoice::Auto => anstream::ColorChoice::Auto,
		clap::ColorChoice::Always => anstream::ColorChoice::Always,
		clap::ColorChoice::Never => anstream::ColorChoice::Never,
	});
	crate::init_logging();

	let matches = Cli::command().color(color).get_matches();

	let cli = Cli::from_arg_matches(&matches).map_err(|error| {
		let _ = error.print();
		2
	})?;

	let executor = resource_management::r#async::Executor::new().map_err(|error| {
		log::error!(
			"Failed to start BELD asynchronous resource access. The most likely cause is that the platform I/O driver could not be initialized. Error: {error}"
		);
		1
	})?;

	executor.block_on(dispatch(cli, parameters, register))
}

/// Dispatches one parsed command through BELD's asynchronous library API.
///
/// Only the commands that read the project resolve its parameters, so a broken `config.json` doesn't stop `list`.
async fn dispatch(cli: Cli, defaults: &[Parameter], register: impl FnOnce(&mut AssetManager, &[Parameter])) -> Result<(), i32> {
	match cli.command {
		Commands::Wipe {} => crate::wipe(cli.destination).await,
		Commands::Clear {} => crate::clear(cli.destination).await,
		Commands::List {} => crate::list(cli.destination).await,
		Commands::Query {
			terms,
			group,
			sort,
			reverse,
			limit,
			format,
		} => {
			let roots = globs(&read_parameters(&cli.config, defaults)?, "resources.roots");
			crate::query(cli.source, cli.destination, roots, terms, group, sort, reverse, limit, format).await
		}
		Commands::Inspect { id, format, payload } => crate::inspect(cli.destination, id, format, payload).await,
		Commands::Browse {} => {
			let roots = globs(&read_parameters(&cli.config, defaults)?, "resources.roots");
			crate::browse(cli.source, cli.destination, roots).await
		}
		Commands::Bake {
			ids,
			memory_budget,
			texture_compression,
			force,
			prune,
			dry_run,
			format,
			payload,
		} => {
			let parameters = read_parameters(&cli.config, defaults)?;
			if dry_run {
				return crate::bake_dry_run(
					cli.source,
					ids,
					bake_memory_budget(memory_budget),
					&parameters,
					register,
					format.unwrap_or(OutputFormat::Human),
					payload.unwrap_or_default(),
					&mut std::io::stdout().lock(),
				)
				.await;
			}
			crate::bake(
				cli.source,
				cli.destination,
				ids,
				cli.storage_mode.map(Into::into),
				texture_compression.map(Into::into),
				bake_memory_budget(memory_budget),
				force,
				&parameters,
				register,
				prune,
			)
			.await
		}
		Commands::Delete { ids } => crate::delete(cli.destination, ids).await,
	}
}

/// Returns the application's `defaults` overridden by the project configuration at `path` and by `BE_*` variables, as
/// the application resolves them at startup, logging the failure and returning exit code 1 when the file can't be read.
fn read_parameters(path: &str, defaults: &[Parameter]) -> Result<Vec<Parameter>, i32> {
	let configuration = read_configuration_file(std::path::Path::new(path)).map_err(|error| {
		log::error!("Failed to read the project configuration. {error}");
		1
	})?;
	Ok(layer_parameters(defaults, configuration, std::env::vars()))
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

/// Returns the configured bake budget, or half of the available system memory, and never less than a quarter of the
/// installed memory.
///
/// The floor matters on macOS, where the available figure subtracts compressed pages and reaches zero under memory
/// pressure. A zero budget pauses every independent bake while any other bake retains memory, which serializes the
/// whole run. The budget is soft, so the floor only bounds how much work starts at once.
fn bake_memory_budget(configured: Option<NonZeroUsize>) -> NonZeroUsize {
	configured.unwrap_or_else(|| {
		let mut system = sysinfo::System::new();
		system.refresh_memory();
		let available_bytes = system.available_memory().max(system.total_memory() / 2);
		let budget_bytes = usize::try_from(available_bytes / 2).unwrap_or(usize::MAX);
		NonZeroUsize::new(budget_bytes).unwrap_or(NonZeroUsize::MIN)
	})
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

	/// The project's configuration file, which the application also reads at startup.
	/// It and `BE_*` environment variables override the application's own parameters, as at startup.
	/// `bake` reads the render feature flags, such as `render.gtao.enabled`, and `query` and `browse` read the
	/// `resources.roots` glob patterns that `is:unreachable` starts from.
	/// Example: `beld --config config.json browse`
	#[arg(long, global = true, default_value = "config.json")]
	config: String,

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
	/// Print the resources that match a filter, grouped and sorted. It takes the filter and fields of `browse`.
	Query {
		/// Filter terms, all of which a resource must match. Put `--` before the terms when one starts with `-`.
		/// Example: `beld query --group class -- 'stored>1MiB' -is:compressed`
		terms: Vec<String>,
		/// The field to group resources by.
		/// Example: `beld query class=Image --group model.format`
		#[arg(long)]
		group: Option<String>,
		/// The field to order groups and resources by. Defaults to `stored`, largest first.
		/// Example: `beld query --sort baked`
		#[arg(long)]
		sort: Option<String>,
		/// Reverses the order of groups and resources.
		#[arg(long)]
		reverse: bool,
		/// The most resources to list in each group. Counts and sums still cover every match.
		#[arg(long)]
		limit: Option<usize>,
		#[arg(long, value_enum, default_value_t = OutputFormat::Human)]
		format: OutputFormat,
	},
	/// Browse every resource interactively: filter, group, and sort them, and follow their sources and references.
	/// Needs an interactive terminal. Example: `beld browse`
	Browse {},
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
		/// After a full bake, delete every stored resource that no root in `resources.roots` reaches.
		/// Needs roots and a debug build, and deletes nothing when an asset fails to bake.
		/// Example: `beld bake --prune`
		#[arg(long, conflicts_with_all = ["ids", "dry_run"])]
		prune: bool,
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

use std::{num::NonZeroUsize, process::ExitCode};

use byte_engine::application::{
	Parameter,
	parameters::{layer_parameters, read_configuration_file},
};
use clap::{
	CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum,
	builder::styling::{AnsiColor, Effects, Styles},
};
use resource_management::asset::manager::AssetManager;

use crate::{OutputFormat, Payload, utils::globs};
