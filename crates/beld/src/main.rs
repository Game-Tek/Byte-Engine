use clap::{Parser, Subcommand, ValueEnum};

mod commands;
mod utils;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
	/// The full path to the assets directory.
	/// Example: `beld --source assets`
	#[arg(short, long, default_value = "assets")]
	source: String,

	/// The full path to the resources directory.
	/// Example: `beld --destination resources`
	#[arg(short, long, default_value = "resources")]
	destination: String,

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

fn main() -> Result<(), i32> {
	let _ = simple_logger::SimpleLogger::new().env().init();

	let cli = Cli::parse();

	let command = cli.command;

	let source_path = cli.source;
	let destination_path = cli.destination;

	match command {
		Commands::Wipe {} => commands::wipe(destination_path),
		Commands::Clear {} => commands::wipe(destination_path),
		Commands::List {} => commands::list(destination_path),
		Commands::Inspect { id, format } => commands::inspect(destination_path, id, format),
		Commands::Bake { ids } => commands::bake(source_path, destination_path, ids),
		Commands::Delete { ids } => commands::delete(destination_path, ids),
	}
}
