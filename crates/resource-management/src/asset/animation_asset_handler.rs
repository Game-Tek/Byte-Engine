use std::sync::Arc;

use crate::resources::animation::{
    AnimationChannel, AnimationModel, AnimationSampler, SamplerOutput,
};
use crate::{
    asset,
    asset::asset_handler::{Asset, AssetHandler, BoxFuture, LoadErrors},
    asset::asset_manager::AssetManager,
    asset::ResourceId,
    resource, Description, ProcessedAsset,
};

use utils::json;

use super::tasks::spawn_cpu_task;

struct AnimationAsset {
    id: String,
    spec: Option<json::Value>,
    gltf: Arc<gltf::Gltf>,
    buffers: Arc<Vec<gltf::buffer::Data>>,
}

impl Asset for AnimationAsset {
    fn requested_assets(&self) -> Vec<String> {
        vec![]
    }

    fn load<'a>(
        &'a self,
        _asset_manager: &'a AssetManager,
        storage_backend: &'a dyn resource::StorageBackend,
        _asset_storage_backend: &'a dyn asset::StorageBackend,
        url: ResourceId<'a>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let gltf = self.gltf.clone();
            let buffers = self.buffers.clone();

            // Check if we need to load a specific animation by fragment
            let target_name = url.get_fragment().map(|f| f.as_ref().to_string());

            let animation_resource = spawn_cpu_task(move || -> Result<AnimationModel, String> {
                let gltf = gltf.as_ref();
                let buffers = buffers.as_ref();

                // Find the animation to load
                let animation = if let Some(ref name) = target_name {
                    gltf.animations()
                        .find(|a| a.name() == Some(name.as_str()))
                        .ok_or_else(|| format!("Animation '{}' not found. The glTF file likely does not contain this animation.", name))?
                } else {
                    // If no fragment specified, load the first animation
                    gltf.animations()
                        .next()
                        .ok_or("No animations found. The glTF file likely contains no animation data.".to_string())?
                };

                let name = animation.name().map(|s| s.to_string());
                let mut samplers = Vec::new();
                let mut max_duration = 0.0f32;

                for sampler in animation.samplers() {
                    let input_accessor = sampler.input();
                    let output_accessor = sampler.output();

                    // Read input times
                    let input_times = read_f32_accessor(&input_accessor, buffers)?;

                    // Update max duration
                    if let Some(&last_time) = input_times.last() {
                        max_duration = max_duration.max(last_time);
                    }

                    // Read output values based on accessor type
                    let output_values = read_output_accessor(&output_accessor, buffers)?;

                    samplers.push(AnimationSampler {
                        interpolation: sampler.interpolation().into(),
                        input_times,
                        output_values,
                    });
                }

                let mut channels = Vec::new();

                for channel in animation.channels() {
                    let sampler_index = channel.sampler().index();
                    let target = channel.target();
                    let target_node = target.node().index();
                    let target_path = target.property().into();

                    channels.push(AnimationChannel {
                        sampler_index,
                        target_node,
                        target_path,
                    });
                }

                Ok(AnimationModel {
                    name: name.clone(),
                    samplers,
                    channels,
                    duration: max_duration,
                })
            })
            .await?;

            let resource_document = ProcessedAsset::new(url, animation_resource);
            storage_backend
                .store(&resource_document, &[])
                .map_err(|_| "Failed to store animation resource. The storage backend likely rejected the write.".to_string())?;

            Ok(())
        })
    }
}

/// Read f32 values from a glTF accessor
fn read_f32_accessor(
    accessor: &gltf::Accessor,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<f32>, String> {
    let view = accessor.view().ok_or("Accessor has no buffer view")?;
    let buffer = &buffers[view.buffer().index()];
    let offset = view.offset() + accessor.offset();
    let count = accessor.count();

    match accessor.dimensions() {
        gltf::accessor::Dimensions::Scalar => match accessor.data_type() {
            gltf::accessor::DataType::F32 => {
                let data = &buffer.0[offset..offset + count * 4];
                let values: Vec<f32> = data
                    .chunks_exact(4)
                    .map(|chunk| {
                        let bytes: [u8; 4] = chunk.try_into().unwrap();
                        f32::from_le_bytes(bytes)
                    })
                    .collect();
                Ok(values)
            }
            _ => Err("Unsupported data type for f32 accessor".to_string()),
        },
        _ => Err("Expected scalar dimensions for f32 accessor".to_string()),
    }
}

/// Read output values from a glTF accessor based on expected output type
fn read_output_accessor(
    accessor: &gltf::Accessor,
    buffers: &[gltf::buffer::Data],
) -> Result<SamplerOutput, String> {
    let view = accessor.view().ok_or("Accessor has no buffer view")?;
    let buffer = &buffers[view.buffer().index()];
    let offset = view.offset() + accessor.offset();
    let count = accessor.count();

    match accessor.dimensions() {
        gltf::accessor::Dimensions::Vec3 => match accessor.data_type() {
            gltf::accessor::DataType::F32 => {
                let data = &buffer.0[offset..offset + count * 12];
                let values: Vec<[f32; 3]> = data
                    .chunks_exact(12)
                    .map(|chunk| {
                        let x = f32::from_le_bytes(chunk[0..4].try_into().unwrap());
                        let y = f32::from_le_bytes(chunk[4..8].try_into().unwrap());
                        let z = f32::from_le_bytes(chunk[8..12].try_into().unwrap());
                        [x, y, z]
                    })
                    .collect();
                Ok(SamplerOutput::Translation(values))
            }
            _ => Err("Unsupported data type for Vec3 accessor".to_string()),
        },
        gltf::accessor::Dimensions::Vec4 => match accessor.data_type() {
            gltf::accessor::DataType::F32 => {
                let data = &buffer.0[offset..offset + count * 16];
                let values: Vec<[f32; 4]> = data
                    .chunks_exact(16)
                    .map(|chunk| {
                        let x = f32::from_le_bytes(chunk[0..4].try_into().unwrap());
                        let y = f32::from_le_bytes(chunk[4..8].try_into().unwrap());
                        let z = f32::from_le_bytes(chunk[8..12].try_into().unwrap());
                        let w = f32::from_le_bytes(chunk[12..16].try_into().unwrap());
                        [x, y, z, w]
                    })
                    .collect();
                Ok(SamplerOutput::Rotation(values))
            }
            _ => Err("Unsupported data type for Vec4 accessor".to_string()),
        },
        gltf::accessor::Dimensions::Scalar => match accessor.data_type() {
            gltf::accessor::DataType::F32 => {
                let data = &buffer.0[offset..offset + count * 4];
                let values: Vec<f32> = data
                    .chunks_exact(4)
                    .map(|chunk| {
                        let bytes: [u8; 4] = chunk.try_into().unwrap();
                        f32::from_le_bytes(bytes)
                    })
                    .collect();
                Ok(SamplerOutput::Weights(values))
            }
            _ => Err("Unsupported data type for scalar accessor".to_string()),
        },
        _ => Err("Unsupported accessor dimensions for animation output".to_string()),
    }
}

/// The `AnimationAssetHandler` handles loading animation data from glTF files.
pub struct AnimationAssetHandler {}

impl AnimationAssetHandler {
    pub fn new() -> AnimationAssetHandler {
        AnimationAssetHandler {}
    }
}

impl AssetHandler for AnimationAssetHandler {
    fn can_handle(&self, r#type: &str) -> bool {
        r#type == "gltf" || r#type == "glb"
    }

    fn load<'a>(
        &'a self,
        _asset_manager: &'a AssetManager,
        storage_backend: &'a dyn resource::StorageBackend,
        asset_storage_backend: &'a dyn asset::StorageBackend,
        url: ResourceId<'a>,
    ) -> BoxFuture<'a, Result<Box<dyn Asset>, LoadErrors>> {
        Box::pin(async move {
            if let Some(dt) = storage_backend.get_type(url) {
                if dt != "gltf" && dt != "glb" {
                    return Err(LoadErrors::UnsupportedType);
                }
            }

            let (data, spec, dt) = asset_storage_backend
                .resolve(url)
                .await
                .or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

            let (gltf, buffers) = if dt == "glb" {
                let glb = gltf::Glb::from_slice(&data).map_err(|_| LoadErrors::FailedToProcess)?;
                let gltf =
                    gltf::Gltf::from_slice(&glb.json).map_err(|_| LoadErrors::FailedToProcess)?;
                let buffers = gltf::import_buffers(
                    &gltf,
                    None,
                    glb.bin.as_ref().map(|b| b.iter().map(|e| *e).collect()),
                )
                .map_err(|_| LoadErrors::FailedToProcess)?;
                (gltf, buffers)
            } else {
                let gltf =
                    gltf::Gltf::from_slice(&data).map_err(|_| LoadErrors::AssetCouldNotBeLoaded)?;

                let buffers = if let Some(bin_file) = gltf.buffers().find_map(|b| {
                    if let gltf::buffer::Source::Uri(r) = b.source() {
                        if r.ends_with(".bin") {
                            Some(r)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }) {
                    let bin_file = ResourceId::new(bin_file);
                    let (bin, ..) = asset_storage_backend
                        .resolve(bin_file)
                        .await
                        .or(Err(LoadErrors::AssetCouldNotBeLoaded))?;
                    gltf.buffers()
                        .map(|_| gltf::buffer::Data(bin.clone().into()))
                        .collect::<Vec<_>>()
                } else {
                    gltf::import_buffers(&gltf, None, None)
                        .map_err(|_| LoadErrors::AssetCouldNotBeLoaded)?
                };

                (gltf, buffers)
            };

            Ok(Box::new(AnimationAsset {
                id: url.to_string(),
                spec,
                gltf: Arc::new(gltf),
                buffers: Arc::new(buffers),
            }) as _)
        })
    }
}

struct AnimationDescription {}

impl Description for AnimationDescription {
    fn get_resource_class() -> &'static str
    where
        Self: Sized,
    {
        "Animation"
    }
}

#[cfg(test)]
mod tests {
    use super::AnimationAssetHandler;
    use crate::{
        asset::{
            self, asset_handler::AssetHandler, asset_manager::AssetManager,
            storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
        },
        resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
        resources::animation::AnimationModel,
        tests::ASSETS_PATH,
        ReferenceModel,
    };

    #[test]
    fn load_gltf_animation() {
        let asset_storage_backend = AssetTestStorageBackend::new();
        let resource_storage_backend = ResourceTestStorageBackend::new();

        let mut asset_manager = AssetManager::new(asset_storage_backend);
        let asset_handler = AnimationAssetHandler::new();
        asset_manager.add_asset_handler(asset_handler);

        let url = "AnimatedCube.gltf";

        let animation: ReferenceModel<AnimationModel> = asset_manager
            .load_sync(url, &resource_storage_backend)
            .expect("Failed to parse asset");

        let generated_resources = resource_storage_backend.get_resources();

        assert_eq!(animation.id().as_ref(), url);
        assert_eq!(animation.class(), "Animation");
    }
}
