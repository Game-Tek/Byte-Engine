use clap::{Parser, Subcommand};
use resource_management::StorageBackend;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// The full path to the resources database. Example: ./resource/resources.db
    #[arg(short, long)]
    path: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
	/// Wipes all resources
	Wipe {},
	/// Lists all resources
	List {},
    /// Deletes a resource
    Delete {
        /// The ID of the resource to delete
        id: String,
    },
}

fn main() -> Result<(), i32> {
    let cli = Cli::parse();

	let command = cli.command;

	let path = cli.path.unwrap_or("resources/resources.db".to_string());

	let storage_backend = resource_management::DbStorageBackend::new(std::path::Path::new(&path));

	match command {
		Commands::Wipe {  } => {
			std::process::Command::new("rm").arg("-rf").arg("resources/*").spawn().unwrap();
			std::process::Command::new("rm").arg("-rf").arg(".byte-editor/*").spawn().unwrap();
			Ok(())
		}
		Commands::List {  } => {
			match smol::block_on(storage_backend.list()) {
				Ok(resources) => {
					if resources.is_empty() {
						println!("No resources found.");
					}

					for resource in resources {
						println!("{}", resource);
					}

					Ok(())
				}
				Err(e) => {
					println!("Failed to list resources. Error: {}", e);
					Err(1)
				}
			}
		}
		Commands::Delete { id } => {
			match smol::block_on(storage_backend.delete(&id)) {
				Ok(()) => {
					println!("Deleted resource '{}'", id);
					Ok(())
				}
				Err(e) => {
					println!("Failed to delete '{}'. Error: {}", id, e);
					Err(1)
				}
			}
		}
	}
}