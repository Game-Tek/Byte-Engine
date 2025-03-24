use clap::{Parser, Subcommand};

use utils::sync::Arc;
use resource_management::{asset::{asset_manager, audio_asset_handler, image_asset_handler, material_asset_handler, mesh_asset_handler}, resource::{ReadStorageBackend, WriteStorageBackend}};

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
	/// Wipe all resources
	Wipe {},
	/// List all resources
	List {},
	/// Bake assets into resources
	Bake {
		/// The IDs of the resources to bake.
		/// Example: `beld bake audio.wav mesh.gltf mesh.gltf#image`
		#[clap(value_delimiter = ' ', num_args = 1..)]
		ids: Vec<String>,
		/// Build resources synchronously
		#[clap(long, default_value = "false")]
		sync: bool,
	},
    /// Delete resources
    Delete {
        /// The IDs of the resources to delete.
		/// Example: `beld delete audio.wav mesh.gltf mesh.gltf#image`
		#[clap(value_delimiter = ' ', num_args = 1..)]
        ids: Vec<String>,
    },
}

fn main() -> Result<(), i32> {
	let _ = simple_logger::SimpleLogger::new().env().init();

    let cli = Cli::parse();

	let command = cli.command;

	let source_path = cli.source;
	let destination_path = cli.destination;

	match command {
		Commands::Wipe {} => {
			std::fs::remove_dir_all(&destination_path).map_err(|e| {
				log::error!("Failed to wipe resources. Error: {}", e);
				1
			})?;

			std::fs::create_dir(&destination_path).map_err(|e| {
				log::error!("Failed to create resources directory. Error: {}", e);
				1
			})?;

			Ok(())
		}
		Commands::List {} => {
			let storage_backend = resource_management::resource::DbStorageBackend::new(destination_path.into());

			match storage_backend.list() {
				Ok(resources) => {
					if resources.is_empty() {
						log::info!("No resources found.");
					}

					for resource in resources {
						println!("{}", resource);
					}

					Ok(())
				}
				Err(e) => {
					log::error!("Failed to list resources. Error: {}", e);
					Err(1)
				}
			}
		}
		Commands::Bake { ids, sync } => {
			let mut asset_manager = asset_manager::AssetManager::new(source_path.into(), destination_path.into());

			asset_manager.add_asset_handler(image_asset_handler::ImageAssetHandler::new());
			asset_manager.add_asset_handler(audio_asset_handler::AudioAssetHandler::new());
			asset_manager.add_asset_handler(mesh_asset_handler::MeshAssetHandler::new());

			{
				let mut material_asset_handler = material_asset_handler::MaterialAssetHandler::new();
				let root_node = besl::Node::root();
				let shader_generator = {
					// let common_shader_generator = byte_engine::rendering::common_shader_generator::CommonShaderGenerator::new();
					let visibility_shader_generation = byte_engine::rendering::visibility_shader_generator::VisibilityShaderGenerator::new(root_node.into());
					visibility_shader_generation
				};
				material_asset_handler.set_shader_generator(shader_generator);
				asset_manager.add_asset_handler(material_asset_handler);
			}

			if ids.is_empty() {
				log::info!("No resources to bake.");
				return Ok(());
			}

			if sync {
				for id in ids {
					log::info!("Baking resource '{}'", id);
					match asset_manager.bake(&id) {
						Ok(_) => {
							log::info!("Baked resource '{}'", id);
						}
						Err(e) => {
							log::error!("Failed to bake '{}'. Error: {:#?}", id, e);
						}
					}
				}
			} else {
				let asset_manager = Arc::new(asset_manager);

				ids.into_iter().for_each(|id| {
					let asset_manager = asset_manager.clone();
					log::info!("Baking resource '{}'", id);
					match asset_manager.bake(&id) {
						Ok(_) => {
							log::info!("Baked resource '{}'", id);
						}
						Err(e) => {
							log::error!("Failed to bake '{}'. Error: {:#?}", id, e);
						}
					}
				});
			}

			Ok(())
		}
		Commands::Delete { ids } => {
			let storage_backend = resource_management::resource::DbStorageBackend::new(destination_path.into());

			let mut ok = true;

			if ids.is_empty() {
				log::info!("No resources to delete.");
				return Ok(());
			}

			for id in ids {
				match storage_backend.delete(resource_management::asset::ResourceId::new(&id)) {
					Ok(()) => {
						log::info!("Deleted resource '{}'", id);
					}
					Err(e) => {
						log::error!("Failed to delete '{}'. Error: {}", id, e);
						ok = false;
					}
				}
			}

			if ok {
				Ok(())
			} else {
				Err(1)
			}
		}
	}
}
