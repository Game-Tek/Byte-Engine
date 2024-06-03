use std::{future, sync::Arc};

use clap::{Parser, Subcommand};
use resource_management::{asset::{self, asset_manager, audio_asset_handler, image_asset_handler, material_asset_handler, mesh_asset_handler}, StorageBackend};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// The full path to the resources directory.
	/// Example: `beld --path resources`
    #[arg(short, long, default_value = "resources")]
    path: String,

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

	let path = cli.path;
	
	match command {
		Commands::Wipe {} => {
			std::fs::remove_dir_all(&path).map_err(|e| {
				log::error!("Failed to wipe resources. Error: {}", e);
				1
			})?;

			std::fs::create_dir(&path).map_err(|e| {
				log::error!("Failed to create resources directory. Error: {}", e);
				1
			})?;
			
			Ok(())
		}
		Commands::List {} => {
			let storage_backend = resource_management::DbStorageBackend::new(std::path::Path::new(&path));

			match smol::block_on(storage_backend.list()) {
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
			let mut asset_manager = asset_manager::AssetManager::new(path.into());

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
					println!("Baking resource '{}'", id);
					match smol::block_on(asset_manager.load(&id)) {
						Ok(_) => {
							println!("Baked resource '{}'", id);
						}
						Err(e) => {
							println!("Failed to bake '{}'. Error: {:#?}", id, e);
						}
					}
				}
			} else {
				let asset_manager = Arc::new(asset_manager);

				tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
					let tasks = ids.into_iter().map(|id| {
						let asset_manager = asset_manager.clone();
						tokio::spawn(async move {
							log::info!("Baking resource '{}'", id);
							match asset_manager.load(&id).await {
								Ok(_) => {
									log::info!("Baked resource '{}'", id);
								}
								Err(e) => {
									log::error!("Failed to bake '{}'. Error: {:#?}", id, e);
								}
							}
						}
					)}).collect::<Vec<_>>();

					futures::future::join_all(tasks).await
				});
			}

			Ok(())
		}
		Commands::Delete { ids } => {
			let storage_backend = resource_management::DbStorageBackend::new(std::path::Path::new(&path));

			let mut ok = true;

			if ids.is_empty() {
				log::info!("No resources to delete.");
				return Ok(());
			}

			for id in ids {
				match smol::block_on(storage_backend.delete(&id)) {
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