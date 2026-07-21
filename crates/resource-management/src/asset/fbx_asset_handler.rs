use std::{
	alloc::Allocator,
	collections::{HashMap, HashSet},
	fmt,
	sync::Arc,
};

use utils::{json, json::JsonValueTrait};

use super::{
	asset_handler::{AssetHandler, BakeContext, LoadErrors},
	asset_manager::AssetManager,
	bema_asset_handler::{compile_shader_program, ProgramGenerator},
	container_default_resource, sanitize_material_name, store_model, ContainerDefaultResource, ResourceId,
};
use crate::{
	asset,
	pbr::{generate_textured_brdf_program, BrdfAlphaMode, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfValue},
	processors::mesh_processor::{
		MeshProcessor, OwnedMeshAttribute, OwnedMeshAttributeData, OwnedMeshPrimitive, OwnedMeshSource,
		TriangleFrontFaceWinding,
	},
	r#async::spawn_cpu_task,
	resource,
	resources::{
		animation::{AnimationModel, NodeTrack, QuaternionCurve, Vector3Curve},
		material::{MaterialModel, RenderModel, Shader, VariantModel},
		skeleton::{LocalTransform, Matrix4Columns, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint, SkinPaletteEntry},
	},
	types::{AlphaMode, VertexComponent, VertexSemantics},
	ProcessedAsset, ReferenceModel,
};

const DEFAULT_ANIMATION_FRAGMENT: &str = "animation";
const ANIMATION_FRAGMENT_PREFIX: &str = "animations/";
const SKELETON_FRAGMENT: &str = "skeleton";
const MAX_PRIMITIVE_VERTICES: usize = u16::MAX as usize + 1;

fn select_unfragmented_fbx_resource(
	scene: &ufbx::Scene,
	spec: Option<&asset::BEADType>,
) -> Result<ContainerDefaultResource, String> {
	let selected = container_default_resource(spec)?;
	if let Some(selected) = selected {
		if selected == ContainerDefaultResource::Animation && scene.anim_stacks.len() != 1 {
			return Err(format!(
				"BEAD selects animation, but the FBX contains {} animation stacks; use an explicit animation fragment",
				scene.anim_stacks.len()
			));
		}
		return Ok(selected);
	}

	if !scene.meshes.is_empty() {
		return Ok(ContainerDefaultResource::Mesh);
	}

	if scene.anim_stacks.len() == 1 {
		return Ok(ContainerDefaultResource::Animation);
	}

	Err(format!(
		"the FBX contains no mesh and {} animation stacks; use an explicit fragment",
		scene.anim_stacks.len()
	))
}

/// The `FBXAssetHandler` struct provides the authored-FBX import path used to bake meshes, skeletons, and animation clips.
#[derive(Default)]
pub struct FBXAssetHandler {
	triangle_front_face_winding: TriangleFrontFaceWinding,
	generator: Option<Arc<dyn ProgramGenerator>>,
}

impl FBXAssetHandler {
	/// Creates an FBX importer using the engine's clockwise mesh-processing convention.
	pub fn new() -> Self {
		Self::default()
	}

	/// Returns the winding convention that will be forwarded to mesh processing.
	pub fn triangle_front_face_winding(&self) -> TriangleFrontFaceWinding {
		self.triangle_front_face_winding
	}

	/// Selects the winding convention used when FBX triangles are packed into mesh streams.
	pub fn set_triangle_front_face_winding(&mut self, winding: TriangleFrontFaceWinding) {
		self.triangle_front_face_winding = winding;
	}

	/// Returns this handler configured with the requested triangle winding convention.
	pub fn with_triangle_front_face_winding(mut self, winding: TriangleFrontFaceWinding) -> Self {
		self.set_triangle_front_face_winding(winding);
		self
	}

	/// Installs the renderer-specific shader transformation used for generated FBX materials.
	pub fn set_shader_generator<G: ProgramGenerator + 'static>(&mut self, generator: G) {
		self.generator = Some(Arc::new(generator));
	}
}

impl AssetHandler for FBXAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("fbx")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(resource_type) = context.resource_type(url) {
			if !self.can_handle(resource_type) {
				return Err(LoadErrors::UnsupportedType);
			}
		}
		let allocator = context.allocator();

		// Resolve the container base so animation fragments never become part of the source filename.
		let base = url.get_base();
		let source_id = ResourceId::new(base.as_ref());
		let (data, spec, source_type) = context.resolve(source_id).await?;
		if !self.can_handle(&source_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		let scene = load_fbx_scene(&data, base.as_ref()).map_err(|error| {
			context.error(format_args!("Failed to import FBX asset '{}': {error}", url.as_ref()));
			LoadErrors::FailedToProcess
		})?;

		if let Some(fragment) = url.get_fragment() {
			let imported_skeleton = import_fbx_skeleton(&scene).map_err(|error| {
				context.error(format_args!("Failed to import FBX skeleton '{}': {error}", url.as_ref()));
				LoadErrors::FailedToProcess
			})?;
			if fragment.as_ref() == SKELETON_FRAGMENT {
				return context.store_primary(ProcessedAsset::new(url, imported_skeleton.model), &[]);
			}

			let skeleton_id = format!("{}#{SKELETON_FRAGMENT}", base.as_ref());
			let skeleton = store_model::<SkeletonModel>(context, &skeleton_id, imported_skeleton.model, &[])?;
			let animation = import_fbx_animation(&scene, fragment.as_ref(), skeleton, &imported_skeleton.source_to_skeleton)
				.map_err(|error| {
					context.error(format_args!("Failed to import FBX animation '{}': {error}", url.as_ref()));
					LoadErrors::FailedToProcess
				})?;
			return context.store_primary(ProcessedAsset::new(url, animation), &[]);
		}

		let default_resource = select_unfragmented_fbx_resource(&scene, spec.as_ref()).map_err(|error| {
			context.error(format_args!(
				"Failed to select the default FBX resource '{}': {error}. The most likely cause is an ambiguous container without an explicit fragment or BEAD override.",
				url.as_ref()
			));
			LoadErrors::FailedToProcess
		})?;

		if default_resource == ContainerDefaultResource::Animation {
			let imported_skeleton = import_fbx_skeleton(&scene).map_err(|error| {
				context.error(format_args!(
					"Failed to import FBX animation skeleton '{}': {error}",
					url.as_ref()
				));
				LoadErrors::FailedToProcess
			})?;

			let skeleton_id = format!("{}#{SKELETON_FRAGMENT}", base.as_ref());
			let skeleton = store_model::<SkeletonModel>(context, &skeleton_id, imported_skeleton.model, &[])?;

			let animation = import_fbx_animation(
				&scene,
				DEFAULT_ANIMATION_FRAGMENT,
				skeleton,
				&imported_skeleton.source_to_skeleton,
			)
			.map_err(|error| {
				context.error(format_args!(
					"Failed to import default FBX animation '{}': {error}",
					url.as_ref()
				));
				LoadErrors::FailedToProcess
			})?;

			return context.store_primary(ProcessedAsset::new(url, animation), &[]);
		}

		let imported_skeleton = (scene.meshes.iter().any(|mesh| !mesh.skin_deformers.is_empty())
			|| !scene.anim_stacks.is_empty())
		.then(|| import_fbx_skeleton(&scene))
		.transpose()
		.map_err(|error| {
			context.error(format_args!("Failed to import FBX skeleton '{}': {error}", url.as_ref()));
			LoadErrors::FailedToProcess
		})?;
		let (skeleton, source_to_skeleton) = if let Some(imported) = imported_skeleton {
			let skeleton_id = format!("{}#{SKELETON_FRAGMENT}", base.as_ref());
			(
				Some(store_model::<SkeletonModel>(context, &skeleton_id, imported.model, &[])?),
				imported.source_to_skeleton,
			)
		} else {
			(None, Vec::new())
		};

		let materials = resolve_fbx_materials(context, spec.as_ref(), source_id, &scene, self.generator.clone()).await?;
		let mut culled_polygons = FbxCulledPolygonCounts::default();
		let source = import_fbx_meshes(
			&scene,
			&materials,
			skeleton,
			&source_to_skeleton,
			allocator,
			&mut culled_polygons,
		);
		culled_polygons.trace(context);
		let source = source.map_err(|error| {
			context.error(format_args!("Failed to import FBX mesh '{}': {error}", url.as_ref()));
			LoadErrors::FailedToProcess
		})?;
		let mesh = MeshProcessor::new()
			.with_triangle_front_face_winding(self.triangle_front_face_winding)
			.process_owned(source)
			.map_err(|error| {
				context.error(format_args!(
					"Failed to process FBX mesh '{}'. The most likely cause is unsupported or malformed mesh data: {error}",
					url.as_ref()
				));
				LoadErrors::FailedToProcess
			})?;

		context.store_primary(
			ProcessedAsset::new(url, mesh.mesh).with_streams(mesh.stream_descriptions),
			&mesh.buffer,
		)
	}
}

/// Loads FBX bytes into ufbx's owned scene while normalizing authored axes and units for Byte-Engine.
fn load_fbx_scene(data: &[u8], filename: &str) -> Result<ufbx::SceneRoot, FbxImportError> {
	ufbx::load_memory(
		data,
		ufbx::LoadOpts {
			filename: ufbx::StringOpt::Ref(filename),
			target_axes: ufbx::CoordinateAxes::left_handed_y_up(),
			target_unit_meters: 1.0,
			handedness_conversion_axis: ufbx::MirrorAxis::Z,
			handedness_conversion_retain_winding: true,
			space_conversion: ufbx::SpaceConversion::AdjustTransforms,
			geometry_transform_handling: ufbx::GeometryTransformHandling::HelperNodes,
			inherit_mode_handling: ufbx::InheritModeHandling::Compensate,
			generate_missing_normals: true,
			clean_skin_weights: true,
			use_blender_pbr_material: true,
			node_depth_limit: 512,
			..Default::default()
		},
	)
	.map_err(|error| FbxImportError::Parse(error.description.to_string()))
}

/// The `ImportedFbxSkeleton` struct keeps source-node remapping beside the stored parent-ordered skeleton.
struct ImportedFbxSkeleton {
	model: SkeletonModel,
	source_to_skeleton: Vec<u32>,
}

/// Imports the adjusted ufbx node tree as the common pose hierarchy used by clips and skin bindings.
fn import_fbx_skeleton(scene: &ufbx::Scene) -> Result<ImportedFbxSkeleton, FbxImportError> {
	if scene.nodes.len() > u32::MAX as usize {
		return Err(FbxImportError::TooManySkeletonNodes);
	}

	let mut nodes = Vec::with_capacity(scene.nodes.len());
	let mut source_to_skeleton = vec![u32::MAX; scene.nodes.len()];
	append_fbx_skeleton_node(&scene.root_node, None, &mut nodes, &mut source_to_skeleton)?;
	if nodes.len() != scene.nodes.len() || source_to_skeleton.contains(&u32::MAX) {
		return Err(FbxImportError::IncompleteSkeleton);
	}

	Ok(ImportedFbxSkeleton {
		model: SkeletonModel { nodes },
		source_to_skeleton,
	})
}

/// Appends one source subtree while assigning remapped parents before their children.
fn append_fbx_skeleton_node(
	node: &ufbx::Node,
	parent: Option<u32>,
	nodes: &mut Vec<SkeletonNode>,
	source_to_skeleton: &mut [u32],
) -> Result<(), FbxImportError> {
	let source_index = node.element.typed_id as usize;
	let mapped = source_to_skeleton
		.get_mut(source_index)
		.ok_or(FbxImportError::InvalidSkeletonNode)?;
	if *mapped != u32::MAX {
		return Err(FbxImportError::DuplicateSkeletonNode);
	}

	let node_index = nodes.len() as u32;
	*mapped = node_index;
	nodes.push(SkeletonNode {
		name: non_empty_name(&node.element.name),
		parent,
		rest_local: local_transform_to_model(node.local_transform)?,
	});
	for child in &node.children {
		append_fbx_skeleton_node(child, Some(node_index), nodes, source_to_skeleton)?;
	}
	Ok(())
}

/// Converts ufbx's adjusted local TRS into the shared CPU-pose representation.
fn local_transform_to_model(transform: ufbx::Transform) -> Result<LocalTransform, FbxImportError> {
	Ok(LocalTransform {
		translation: vec3_to_f32(transform.translation, "skeleton translation")?,
		rotation: quat_to_f32(transform.rotation, "skeleton rotation")?,
		scale: vec3_to_f32(transform.scale, "skeleton scale")?,
	})
}

/// Converts one selected FBX take into sparse node tracks targeting the imported skeleton.
fn import_fbx_animation(
	scene: &ufbx::Scene,
	fragment: &str,
	skeleton: ReferenceModel<SkeletonModel>,
	source_to_skeleton: &[u32],
) -> Result<AnimationModel, FbxImportError> {
	let stack = select_animation_stack(scene, fragment)?;
	let baked = ufbx::bake_anim(
		scene,
		&stack.anim,
		ufbx::BakeOpts {
			trim_start_time: true,
			..Default::default()
		},
	)
	.map_err(|error| FbxImportError::AnimationBake(error.description.to_string()))?;

	let mut tracks = Vec::with_capacity(baked.nodes.len());

	for node in &baked.nodes {
		let target = remap_skeleton_node(source_to_skeleton, node.typed_id)?;
		let translation = import_vec3_curve(&node.translation_keys, "animation translation")?;
		let rotation = import_quaternion_curve(&node.rotation_keys)?;
		let scale = import_vec3_curve(&node.scale_keys, "animation scale")?;
		if translation.is_some() || rotation.is_some() || scale.is_some() {
			tracks.push(NodeTrack {
				node: target,
				translation,
				rotation,
				scale,
			});
		}
	}
	// ufbx sorts baked tracks by source typed ID, while the CPU graph requires dense hierarchy order.
	tracks.sort_unstable_by_key(|track| track.node);

	Ok(AnimationModel {
		name: non_empty_name(&stack.element.name),
		skeleton,
		duration: finite_f32(baked.playback_duration, "animation duration")?,
		tracks,
	})
}

/// Selects the first, indexed, or named animation stack addressed by an FBX resource fragment.
fn select_animation_stack<'a>(scene: &'a ufbx::Scene, fragment: &str) -> Result<&'a ufbx::AnimStack, FbxImportError> {
	if fragment == DEFAULT_ANIMATION_FRAGMENT {
		return scene
			.anim_stacks
			.as_ref()
			.first()
			.map(AsRef::as_ref)
			.ok_or_else(|| FbxImportError::AnimationNotFound("the FBX scene does not contain animation stacks".to_string()));
	}

	let selector = fragment
		.strip_prefix(ANIMATION_FRAGMENT_PREFIX)
		.ok_or_else(|| FbxImportError::UnsupportedFragment(fragment.to_string()))?;
	if selector.is_empty() {
		return Err(FbxImportError::AnimationNotFound(
			"the animation fragment has no index or name".to_string(),
		));
	}

	if let Ok(index) = selector.parse::<usize>() {
		return scene
			.anim_stacks
			.as_ref()
			.get(index)
			.map(AsRef::as_ref)
			.ok_or_else(|| FbxImportError::AnimationNotFound(format!("animation stack index {index} is out of range")));
	}

	scene
		.anim_stacks
		.as_ref()
		.iter()
		.map(AsRef::as_ref)
		.find(|stack| stack.element.name.as_ref() == selector)
		.ok_or_else(|| FbxImportError::AnimationNotFound(format!("animation stack '{selector}' does not exist")))
}

/// Resolves a source typed ID through the dense hierarchy remap shared by clips and skins.
fn remap_skeleton_node(source_to_skeleton: &[u32], source_node: u32) -> Result<u32, FbxImportError> {
	let mapped = source_to_skeleton
		.get(source_node as usize)
		.copied()
		.ok_or(FbxImportError::InvalidSkeletonNode)?;
	(mapped != u32::MAX)
		.then_some(mapped)
		.ok_or(FbxImportError::InvalidSkeletonNode)
}

/// Converts baked vectors directly into a persistent linear curve without transient keyframe objects.
fn import_vec3_curve(keys: &[ufbx::BakedVec3], context: &'static str) -> Result<Option<Vector3Curve>, FbxImportError> {
	if keys.is_empty() {
		return Ok(None);
	}

	let mut times = Vec::with_capacity(keys.len());
	let mut values = Vec::with_capacity(keys.len());
	for key in keys {
		times.push(finite_f32(key.time, "animation key time")?);
		values.push(vec3_to_f32(key.value, context)?);
	}
	Ok(Some(Vector3Curve::Linear { times, values }))
}

/// Converts baked rotations directly into a persistent linear quaternion curve.
fn import_quaternion_curve(keys: &[ufbx::BakedQuat]) -> Result<Option<QuaternionCurve>, FbxImportError> {
	if keys.is_empty() {
		return Ok(None);
	}

	let mut times = Vec::with_capacity(keys.len());
	let mut values = Vec::with_capacity(keys.len());
	for key in keys {
		times.push(finite_f32(key.time, "animation key time")?);
		values.push(quat_to_f32(key.value, "animation quaternion")?);
	}
	Ok(Some(QuaternionCurve::Linear { times, values }))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// The `MaterialKey` enum identifies a generated fallback material or one ufbx scene material.
enum MaterialKey {
	Default,
	Material(u32),
}

/// The `ResolvedFbxMaterials` struct keeps the resource references used while imported material parts are assembled.
struct ResolvedFbxMaterials {
	materials: HashMap<MaterialKey, ReferenceModel<VariantModel>>,
}

impl ResolvedFbxMaterials {
	fn get(&self, key: MaterialKey) -> Result<ReferenceModel<VariantModel>, FbxImportError> {
		self.materials.get(&key).cloned().ok_or(FbxImportError::MissingMaterial)
	}
}

/// Resolves each used FBX material exactly once, honoring `.fbx.bead` overrides before generating a solid fallback.
async fn resolve_fbx_materials(
	context: BakeContext<'_>,
	spec: Option<&json::Value>,
	url: ResourceId<'_>,
	scene: &ufbx::Scene,
	generator: Option<Arc<dyn ProgramGenerator>>,
) -> Result<ResolvedFbxMaterials, LoadErrors> {
	let allocator = context.allocator();
	let keys = used_material_keys(scene, allocator);
	let mut materials = HashMap::with_capacity(keys.len());

	for key in keys {
		let material = match key {
			MaterialKey::Default => None,
			MaterialKey::Material(index) => scene.materials.as_ref().get(index as usize).map(AsRef::as_ref),
		};
		let resolved = if let Some(override_id) = fbx_material_override(spec, material) {
			context.bake_dependency::<VariantModel>(&override_id).await?
		} else {
			generate_fbx_material(context, url, key, material, generator.clone()).await?
		};
		materials.insert(key, resolved);
	}

	Ok(ResolvedFbxMaterials { materials })
}

/// Collects material identities in deterministic first-use order across FBX mesh instances.
fn used_material_keys<'a>(scene: &ufbx::Scene, allocator: &'a dyn Allocator) -> Vec<MaterialKey, &'a dyn Allocator> {
	let mut keys = Vec::with_capacity_in(scene.materials.len().saturating_add(1), allocator);
	let mut seen = HashSet::with_capacity(scene.materials.len().saturating_add(1));
	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};
		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}

		let material_node = authored_material_node(node);
		// Keep first-use ordering stable so generated fallback materials stay deterministic across reimports.
		let mut record_slot = |slot| {
			let key = material_key_for_slot(material_node, mesh, slot);
			if seen.insert(key) {
				keys.push(key);
			}
		};
		if mesh.material_parts.is_empty() {
			if mesh
				.faces
				.iter()
				.enumerate()
				.any(|(index, _)| is_visible_polygon_face(mesh, index))
			{
				record_slot(0);
			}
		} else {
			for part in &mesh.material_parts {
				if part
					.face_indices
					.iter()
					.any(|&index| is_visible_polygon_face(mesh, index as usize))
				{
					record_slot(part.index as usize);
				}
			}
		}
	}
	keys
}

/// Filters point, line, and authored hole faces out of renderable polygon processing.
fn is_visible_polygon_face(mesh: &ufbx::Mesh, face_index: usize) -> bool {
	mesh.faces
		.get(face_index)
		.is_some_and(|face| face.num_indices >= 3 && !mesh.face_hole.get(face_index).copied().unwrap_or(false))
}

/// Resolves a material slot against the preselected authored instance before using the mesh-wide fallback binding.
fn material_key_for_slot(material_node: &ufbx::Node, mesh: &ufbx::Mesh, slot: usize) -> MaterialKey {
	material_node
		.materials
		.as_ref()
		.get(slot)
		.or_else(|| mesh.materials.as_ref().get(slot))
		.map(|material| MaterialKey::Material(material.element.typed_id))
		.unwrap_or(MaterialKey::Default)
}

/// Finds the authored instance node behind ufbx helper nodes so per-instance material bindings remain distinct.
fn authored_material_node(mut node: &ufbx::Node) -> &ufbx::Node {
	while node.is_geometry_transform_helper {
		let Some(parent) = node.parent.as_ref() else {
			break;
		};
		node = parent.as_ref();
	}
	node
}

/// Reads an optional `.fbx.bead` material override by authored material name or the `default` key.
fn fbx_material_override(spec: Option<&json::Value>, material: Option<&ufbx::Material>) -> Option<String> {
	let key = material
		.map(|material| material.element.name.as_ref())
		.filter(|name| !name.is_empty())
		.unwrap_or("default");
	let material = &spec?["asset"][&key];
	material["asset"].as_str().map(ToString::to_string)
}

/// Generates the current solid-value subset of an FBX material and stores its shader/material/variant resource chain.
async fn generate_fbx_material(
	context: BakeContext<'_>,
	mesh_url: ResourceId<'_>,
	key: MaterialKey,
	material: Option<&ufbx::Material>,
	generator: Option<Arc<dyn ProgramGenerator>>,
) -> Result<ReferenceModel<VariantModel>, LoadErrors> {
	let generator = generator.ok_or_else(|| {
		context.error(
			"FBX material generation is unavailable. The most likely cause is that the FBX asset handler has no shader generator."
		);
		LoadErrors::FailedToProcess
	})?;
	let brdf = fbx_brdf_material(material);
	let alpha_mode = AlphaMode::from(brdf.alpha_mode);
	let program = generate_textured_brdf_program(&brdf).map_err(|_| LoadErrors::FailedToProcess)?;
	let base_id = generated_fbx_material_base_id(mesh_url, key, material);
	let shader_id = format!("{base_id}.shader");
	let material_id = format!("{base_id}.material");
	let variant_id = format!("{base_id}.variant");
	let shader_name = shader_id.clone();
	let material_json = json::object! { "variables": Vec::<json::Value>::new() };

	let (shader, shader_bytes) = spawn_cpu_task(move || {
		compile_shader_program(generator.as_ref(), &shader_name, program, "World", &material_json, "Compute")
	})
	.await
	.map_err(|_| {
		context.error(
			"FBX material shader compilation did not complete. The most likely cause is a failed background compiler task.",
		);
		LoadErrors::FailedToProcess
	})?
	.map_err(|_| {
		context.error(format_args!(
			"Failed to compile generated FBX material shader '{shader_id}'. The most likely cause is an invalid generated shader or unavailable platform compiler."
		));
		LoadErrors::FailedToProcess
	})?;

	let shader = store_model::<Shader>(context, &shader_id, shader, &shader_bytes)?;
	let material = MaterialModel {
		double_sided: brdf.double_sided,
		alpha_mode: alpha_mode.clone(),
		model: RenderModel {
			name: "Visibility".to_string(),
			pass: "MaterialEvaluation".to_string(),
		},
		shaders: vec![shader],
		parameters: Vec::new(),
	};
	let material = store_model::<MaterialModel>(context, &material_id, material, &[])?;
	let variant = VariantModel {
		material,
		variables: Vec::new(),
		alpha_mode,
	};
	store_model::<VariantModel>(context, &variant_id, variant, &[])
}

/// Converts ufbx's normalized PBR values into the engine's solid metallic-roughness graph.
fn fbx_brdf_material(material: Option<&ufbx::Material>) -> crate::pbr::BrdfMaterialDescription {
	let mut builder = BrdfMaterialBuilder::new();
	let (name, base_color, metallic, roughness, emission, double_sided) = if let Some(material) = material {
		let base_factor = material_map_scalar(&material.pbr.base_factor, 1.0).clamp(0.0, 1.0);
		let mut base_color = material_map_vec4(
			&material.pbr.base_color,
			material_map_vec4(&material.fbx.diffuse_color, [1.0; 4]),
		);
		for component in &mut base_color[..3] {
			*component = finite_material_product(component.clamp(0.0, 1.0), base_factor, 1.0);
		}
		base_color[3] = finite_material_product(base_color[3].clamp(0.0, 1.0), material_opacity(material), 1.0);
		let emission_factor = material_map_scalar(&material.pbr.emission_factor, 1.0).max(0.0);
		let emission = multiply_vec3(
			material_map_vec3(
				&material.pbr.emission_color,
				material_map_vec3(&material.fbx.emission_color, [0.0; 3]),
			),
			[emission_factor; 3],
		);
		(
			non_empty_name(&material.element.name),
			base_color,
			material_map_scalar(&material.pbr.metalness, 0.0).clamp(0.0, 1.0),
			material_map_scalar(&material.pbr.roughness, 1.0).clamp(0.0, 1.0),
			emission,
			material.features.double_sided.enabled,
		)
	} else {
		(None, [1.0; 4], 0.0, 1.0, [0.0; 3], false)
	};

	let base_color_node = builder.constant(BrdfValue::Vector4(base_color));
	let metallic_node = builder.constant(BrdfValue::Scalar(metallic));
	let roughness_node = builder.constant(BrdfValue::Scalar(roughness));
	let emission_color = builder.constant(BrdfValue::Vector3(emission));
	let emission_node = builder.add(BrdfNode::Emission { color: emission_color });
	let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
		base_color: base_color_node,
		metallic: metallic_node,
		roughness: roughness_node,
		normal: None,
		occlusion: None,
		emission: Some(emission_node),
	}));
	let alpha_mode = if base_color[3] < 0.999 {
		BrdfAlphaMode::Blend
	} else {
		BrdfAlphaMode::Opaque
	};
	builder.finish(name, surface, double_sided, alpha_mode)
}

/// Resolves explicit PBR opacity or derives it from FBX transparency for legacy Phong materials.
fn material_opacity(material: &ufbx::Material) -> f32 {
	if material.pbr.opacity.has_value {
		return material_map_scalar(&material.pbr.opacity, 1.0).clamp(0.0, 1.0);
	}
	let transparency = if material.pbr.transmission_factor.has_value {
		material_map_scalar(&material.pbr.transmission_factor, 0.0)
	} else {
		material_map_scalar(&material.fbx.transparency_factor, 0.0)
	};
	(1.0 - transparency).clamp(0.0, 1.0)
}

/// Reads the scalar x component used by ufbx material factor maps.
fn material_map_scalar(map: &ufbx::MaterialMap, default: f32) -> f32 {
	if map.has_value {
		finite_material_component(map.value_vec4.x, default)
	} else {
		default
	}
}

/// Reads a three-component ufbx material color with finite fallbacks per component.
fn material_map_vec3(map: &ufbx::MaterialMap, default: [f32; 3]) -> [f32; 3] {
	if map.has_value {
		[
			finite_material_component(map.value_vec4.x, default[0]),
			finite_material_component(map.value_vec4.y, default[1]),
			finite_material_component(map.value_vec4.z, default[2]),
		]
	} else {
		default
	}
}

/// Reads a four-component ufbx material color with finite fallbacks per component.
fn material_map_vec4(map: &ufbx::MaterialMap, default: [f32; 4]) -> [f32; 4] {
	if map.has_value {
		[
			finite_material_component(map.value_vec4.x, default[0]),
			finite_material_component(map.value_vec4.y, default[1]),
			finite_material_component(map.value_vec4.z, default[2]),
			finite_material_component(map.value_vec4.w, default[3]),
		]
	} else {
		default
	}
}

/// Converts a material component without allowing f64 values that overflow the engine's f32 representation.
fn finite_material_component(value: f64, default: f32) -> f32 {
	if value.is_finite() && value >= f32::MIN as f64 && value <= f32::MAX as f64 {
		value as f32
	} else {
		default
	}
}

/// Multiplies non-negative material colors while replacing overflow with a safe fallback.
fn multiply_vec3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		finite_material_product(left[0].max(0.0), right[0].max(0.0), 0.0),
		finite_material_product(left[1].max(0.0), right[1].max(0.0), 0.0),
		finite_material_product(left[2].max(0.0), right[2].max(0.0), 0.0),
	]
}

/// Computes a material factor product at f64 precision before checking that it fits in f32.
fn finite_material_product(left: f32, right: f32, default: f32) -> f32 {
	finite_material_component(left as f64 * right as f64, default)
}

/// Builds a deterministic, collision-resistant resource prefix for a generated FBX material chain.
fn generated_fbx_material_base_id(mesh_url: ResourceId<'_>, key: MaterialKey, material: Option<&ufbx::Material>) -> String {
	let index = match key {
		MaterialKey::Default => "default".to_string(),
		MaterialKey::Material(index) => index.to_string(),
	};
	let name = material
		.and_then(|material| non_empty_name(&material.element.name))
		.map(|name| sanitize_material_name(&name))
		.unwrap_or_else(|| "material".to_string());
	format!("{}#materials/{index}_{name}", mesh_url.as_ref())
}

/// The `VertexAttributeMask` struct keeps fixed semantic availability reusable across FBX mesh-import loops.
#[derive(Clone, Copy, Default)]
struct VertexAttributeMask(u8);

impl VertexAttributeMask {
	/// Captures authored FBX attribute availability once for all primitive batches of a mesh instance.
	fn from_mesh(mesh: &ufbx::Mesh) -> Self {
		let mut attributes = Self::default();
		if mesh.vertex_normal.exists {
			attributes.insert(VertexSemantics::Normal);
		}
		if mesh.vertex_tangent.exists {
			attributes.insert(VertexSemantics::Tangent);
		}
		if mesh.vertex_bitangent.exists {
			attributes.insert(VertexSemantics::BiTangent);
		}
		if mesh.vertex_uv.exists {
			attributes.insert(VertexSemantics::UV);
		}
		if mesh.vertex_color.exists {
			attributes.insert(VertexSemantics::Color);
		}
		attributes
	}

	fn contains(self, semantic: VertexSemantics) -> bool {
		self.0 & vertex_semantic_bit(semantic) != 0
	}

	fn insert(&mut self, semantic: VertexSemantics) -> bool {
		let bit = vertex_semantic_bit(semantic);
		let inserted = self.0 & bit == 0;
		self.0 |= bit;
		inserted
	}
}

/// Maps the engine's fixed vertex semantics to compact importer state.
const fn vertex_semantic_bit(semantic: VertexSemantics) -> u8 {
	match semantic {
		VertexSemantics::Position => 1 << 0,
		VertexSemantics::Normal => 1 << 1,
		VertexSemantics::Tangent => 1 << 2,
		VertexSemantics::BiTangent => 1 << 3,
		VertexSemantics::UV => 1 << 4,
		VertexSemantics::Color => 1 << 5,
		VertexSemantics::Joints => 1 << 6,
		VertexSemantics::Weights => 1 << 7,
	}
}

/// The `FbxMeshImportContext` struct carries per-instance data shared by every material part and primitive batch.
struct FbxMeshImportContext<'a> {
	node: &'a ufbx::Node,
	mesh: &'a ufbx::Mesh,
	material_node: &'a ufbx::Node,
	normal_matrix: Option<ufbx::Matrix>,
	source_attributes: VertexAttributeMask,
	skin: Option<&'a ufbx::SkinDeformer>,
	transform_node: Option<u32>,
	skin_index: Option<u32>,
	fallback_joint: Option<u16>,
	mirrored: bool,
}

impl<'a> FbxMeshImportContext<'a> {
	/// Builds reusable instance state and validates invariants before primitive batches are extracted.
	fn new(
		node: &'a ufbx::Node,
		mesh: &'a ufbx::Mesh,
		skin: Option<&'a ufbx::SkinDeformer>,
		transform_node: Option<u32>,
		skin_index: Option<u32>,
		fallback_joint: Option<u16>,
	) -> Result<Self, FbxImportError> {
		let determinant = ufbx::matrix_determinant(&node.geometry_to_world);
		if !determinant.is_finite() {
			return Err(FbxImportError::NonFinite("mesh instance transform determinant"));
		}
		if transform_node.is_some() && determinant.abs() <= f64::EPSILON {
			return Err(FbxImportError::NonInvertibleAnimatedMeshTransform);
		}

		let source_attributes = VertexAttributeMask::from_mesh(mesh);
		let normal_matrix = source_attributes
			.contains(VertexSemantics::Normal)
			.then(|| ufbx::matrix_for_normals(&node.geometry_to_world));
		Ok(Self {
			node,
			mesh,
			material_node: authored_material_node(node),
			normal_matrix,
			source_attributes,
			skin,
			transform_node,
			skin_index,
			fallback_joint,
			mirrored: determinant < 0.0,
		})
	}
}

/// Selects the single linear or rigid deformer supported by the fixed matrix-palette contract.
fn select_fbx_skin(mesh: &ufbx::Mesh) -> Result<Option<&ufbx::SkinDeformer>, FbxImportError> {
	if mesh.skin_deformers.len() > 1 {
		return Err(FbxImportError::MultipleSkinDeformers);
	}
	let Some(skin) = mesh.skin_deformers.as_ref().first().map(AsRef::as_ref) else {
		return Ok(None);
	};
	if matches!(
		skin.skinning_method,
		ufbx::SkinningMethod::DualQuaternion | ufbx::SkinningMethod::BlendedDqLinear
	) || skin.vertices.iter().any(|vertex| vertex.dq_weight > 0.0)
	{
		return Err(FbxImportError::UnsupportedDualQuaternionSkinning);
	}
	Ok(Some(skin))
}

/// Builds one mesh-instance palette and adjusts inverse binds for the importer's flattened vertex space.
fn import_fbx_skin_binding(
	node: &ufbx::Node,
	skin: &ufbx::SkinDeformer,
	source_to_skeleton: &[u32],
) -> Result<(SkinBinding, Option<u16>), FbxImportError> {
	let determinant = ufbx::matrix_determinant(&node.geometry_to_world);
	if !determinant.is_finite() {
		return Err(FbxImportError::NonFinite("skinned mesh transform determinant"));
	}
	if determinant.abs() <= f64::EPSILON {
		return Err(FbxImportError::NonInvertibleSkinTransform);
	}

	let mut needs_fallback = false;
	for vertex in 0..skin.vertices.len() {
		if strongest_skin_weight_total(skin, vertex)? == 0.0 {
			needs_fallback = true;
			break;
		}
	}
	let palette_len = skin.clusters.len().saturating_add(usize::from(needs_fallback));
	if palette_len > MAX_PRIMITIVE_VERTICES {
		return Err(FbxImportError::TooManyJoints);
	}

	let geometry_world_inverse = ufbx::matrix_invert(&node.geometry_to_world);
	let mut entries = Vec::with_capacity(palette_len);
	for cluster in &skin.clusters {
		let bone = cluster.bone_node.as_ref().ok_or(FbxImportError::MissingSkinBone)?;
		// Vertices already contain `geometry_to_world`, so remove that flattened bind transform after
		// ufbx's geometry-to-bone matrix. A runtime global bone matrix can then produce the final palette.
		let adjusted = ufbx::matrix_mul(&cluster.geometry_to_bone, &geometry_world_inverse);
		entries.push(SkinPaletteEntry {
			joint: SkinJoint::Node(remap_skeleton_node(source_to_skeleton, bone.element.typed_id)?),
			adjusted_inverse_bind_matrix: matrix_to_columns(&adjusted)?,
		});
	}

	let fallback_joint = if needs_fallback {
		let index = u16::try_from(entries.len()).map_err(|_| FbxImportError::TooManyJoints)?;
		// ufbx evaluates an unweighted control point with the mesh instance transform. Binding the
		// fallback entry to that node preserves the behavior when the mesh or an ancestor animates.
		entries.push(SkinPaletteEntry {
			joint: SkinJoint::Node(remap_skeleton_node(source_to_skeleton, node.element.typed_id)?),
			adjusted_inverse_bind_matrix: matrix_to_columns(&geometry_world_inverse)?,
		});
		Some(index)
	} else {
		None
	};

	Ok((SkinBinding { entries }, fallback_joint))
}

/// Sums the retained fixed-width influences without allocating temporary weight storage.
fn strongest_skin_weight_total(skin: &ufbx::SkinDeformer, logical_vertex: usize) -> Result<f64, FbxImportError> {
	let influences = skin_influences(skin, logical_vertex)?;
	let mut total = 0.0;
	for influence in influences.iter().take(4) {
		if influence.cluster_index as usize >= skin.clusters.len() {
			return Err(FbxImportError::InvalidSkinCluster);
		}
		total += finite_f32(influence.weight, "skin weight")?.max(0.0) as f64;
	}
	Ok(total)
}

/// Borrows one logical vertex's sorted ufbx influence range after validating its bounds.
fn skin_influences(skin: &ufbx::SkinDeformer, logical_vertex: usize) -> Result<&[ufbx::SkinWeight], FbxImportError> {
	let vertex = skin.vertices.get(logical_vertex).ok_or(FbxImportError::InvalidSkinVertex)?;
	let begin = vertex.weight_begin as usize;
	let end = begin
		.checked_add(vertex.num_weights as usize)
		.ok_or(FbxImportError::InvalidSkinVertex)?;
	skin.weights.get(begin..end).ok_or(FbxImportError::InvalidSkinVertex)
}

/// Converts ufbx's affine column vectors into the serialized four-column matrix representation.
fn matrix_to_columns(matrix: &ufbx::Matrix) -> Result<Matrix4Columns, FbxImportError> {
	Ok([
		[
			finite_f32(matrix.m00, "skin matrix")?,
			finite_f32(matrix.m10, "skin matrix")?,
			finite_f32(matrix.m20, "skin matrix")?,
			0.0,
		],
		[
			finite_f32(matrix.m01, "skin matrix")?,
			finite_f32(matrix.m11, "skin matrix")?,
			finite_f32(matrix.m21, "skin matrix")?,
			0.0,
		],
		[
			finite_f32(matrix.m02, "skin matrix")?,
			finite_f32(matrix.m12, "skin matrix")?,
			finite_f32(matrix.m22, "skin matrix")?,
			0.0,
		],
		[
			finite_f32(matrix.m03, "skin matrix")?,
			finite_f32(matrix.m13, "skin matrix")?,
			finite_f32(matrix.m23, "skin matrix")?,
			1.0,
		],
	])
}

/// The `FbxMeshAllocationEstimates` struct carries scene-derived capacities for reusable importer buffers.
struct FbxMeshAllocationEstimates {
	primitives: usize,
	scratch: usize,
	corners: usize,
	remap: usize,
}

/// Estimates common-case primitive count and worst-case reusable scratch sizes from ufbx metadata.
fn fbx_mesh_allocation_estimates(scene: &ufbx::Scene) -> FbxMeshAllocationEstimates {
	let mut estimates = FbxMeshAllocationEstimates {
		primitives: 0,
		scratch: 3,
		corners: 0,
		remap: 0,
	};
	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};
		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}

		estimates.scratch = estimates.scratch.max(mesh.max_face_triangles.saturating_mul(3));
		estimates.remap = estimates.remap.max(mesh.num_indices);
		let (mesh_corners, mesh_primitives) = if mesh.material_parts.is_empty() {
			let corners = mesh.num_triangles.saturating_mul(3);
			(corners, corners.div_ceil(MAX_PRIMITIVE_VERTICES).max(1))
		} else {
			let corners = mesh
				.material_parts
				.iter()
				.map(|part| part.num_triangles.saturating_mul(3))
				.max()
				.unwrap_or(0);
			let primitives = mesh.material_parts.iter().fold(0usize, |count, part| {
				count.saturating_add(part.num_triangles.saturating_mul(3).div_ceil(MAX_PRIMITIVE_VERTICES).max(1))
			});
			(corners, primitives)
		};
		estimates.primitives = estimates.primitives.saturating_add(mesh_primitives);
		estimates.corners = estimates.corners.max(mesh_corners);
	}
	estimates
}

/// Imports every mesh instance and material part into processor-owned, per-corner vertex data.
fn import_fbx_meshes<'a>(
	scene: &ufbx::Scene,
	materials: &ResolvedFbxMaterials,
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	source_to_skeleton: &[u32],
	allocator: &'a dyn Allocator,
	culled_polygons: &mut FbxCulledPolygonCounts,
) -> Result<OwnedMeshSource<&'a dyn Allocator>, FbxImportError> {
	let estimates = fbx_mesh_allocation_estimates(scene);
	let mut layout = Vec::with_capacity_in(8, allocator);
	let mut layout_semantics = VertexAttributeMask::default();
	let mut primitives = Vec::with_capacity_in(estimates.primitives, allocator);
	let mut scratch = Vec::with_capacity_in(estimates.scratch, allocator);
	let mut corners = Vec::with_capacity_in(estimates.corners, allocator);
	let mut remap = Vec::with_capacity_in(estimates.remap, allocator);
	let skin_capacity = scene
		.nodes
		.iter()
		.filter_map(|node| node.mesh.as_ref())
		.filter(|mesh| !mesh.skin_deformers.is_empty())
		.count();
	let mut skins = Vec::with_capacity(skin_capacity);

	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};
		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}
		let skin = select_fbx_skin(mesh)?;
		let (skin_index, fallback_joint) = if let Some(skin) = skin {
			let skin_index = u32::try_from(skins.len()).map_err(|_| FbxImportError::TooManySkinBindings)?;
			let (binding, fallback_joint) = import_fbx_skin_binding(node, skin, source_to_skeleton)?;
			skins.push(binding);
			(Some(skin_index), fallback_joint)
		} else {
			(None, None)
		};
		let transform_node = if source_to_skeleton.is_empty() {
			None
		} else {
			Some(remap_skeleton_node(source_to_skeleton, node.element.typed_id)?)
		};
		let context = FbxMeshImportContext::new(node, mesh, skin, transform_node, skin_index, fallback_joint)?;

		// Reuse triangulation and corner-remap storage across mesh instances and material parts to bound import allocations.
		let scratch_len = mesh.max_face_triangles.saturating_mul(3).max(3);
		scratch.resize(scratch_len, 0u32);
		corners.clear();
		remap.clear();
		remap.resize(mesh.num_indices, u32::MAX);

		if mesh.material_parts.is_empty() {
			corners.reserve(mesh.num_triangles.saturating_mul(3));
			for (face_index, &face) in mesh.faces.iter().enumerate() {
				if !is_visible_polygon_face(mesh, face_index) {
					continue;
				}
				if append_triangulated_face(mesh, face, &mut scratch, &mut corners)?
					== TriangulatedFaceAppendResult::CulledDegenerate
				{
					culled_polygons.record(face.num_indices);
				}
			}
			import_fbx_material_corners(
				&context,
				0,
				&corners,
				&mut remap,
				materials,
				&mut layout,
				&mut layout_semantics,
				&mut primitives,
				allocator,
			)?;
		} else {
			for part in &mesh.material_parts {
				corners.clear();
				let required_capacity = part.num_triangles.saturating_mul(3);
				if corners.capacity() < required_capacity {
					corners.reserve(required_capacity.saturating_sub(corners.len()));
				}
				for &face_index in &part.face_indices {
					let face = mesh
						.faces
						.get(face_index as usize)
						.copied()
						.ok_or(FbxImportError::InvalidFaceIndex)?;
					if face.num_indices < 3 || mesh.face_hole.get(face_index as usize).copied().unwrap_or(false) {
						continue;
					}
					if append_triangulated_face(mesh, face, &mut scratch, &mut corners)?
						== TriangulatedFaceAppendResult::CulledDegenerate
					{
						culled_polygons.record(face.num_indices);
					}
				}
				import_fbx_material_corners(
					&context,
					part.index as usize,
					&corners,
					&mut remap,
					materials,
					&mut layout,
					&mut layout_semantics,
					&mut primitives,
					allocator,
				)?;
			}
		}
	}
	if primitives.is_empty() {
		return Err(FbxImportError::NoMesh);
	}
	let mut source = OwnedMeshSource::new(layout, primitives).with_skins(skins);
	source.set_skeleton(skeleton);
	Ok(source)
}

/// Imports one triangulated material part immediately so source-corner storage can be reused by the next part.
fn import_fbx_material_corners<'a>(
	context: &FbxMeshImportContext<'_>,
	material_slot: usize,
	corners: &[u32],
	remap: &mut [u32],
	materials: &ResolvedFbxMaterials,
	layout: &mut Vec<VertexComponent, &'a dyn Allocator>,
	layout_semantics: &mut VertexAttributeMask,
	primitives: &mut Vec<OwnedMeshPrimitive<&'a dyn Allocator>, &'a dyn Allocator>,
	allocator: &'a dyn Allocator,
) -> Result<(), FbxImportError> {
	if corners.is_empty() {
		return Ok(());
	}
	let material = materials.get(material_key_for_slot(context.material_node, context.mesh, material_slot))?;
	for batch in remap_triangle_corners(context.mesh.num_indices, corners, remap, allocator)? {
		primitives.push(import_fbx_primitive(
			context,
			material.clone(),
			batch,
			layout,
			layout_semantics,
			allocator,
		)?);
	}
	Ok(())
}

/// The `TriangulatedFaceAppendResult` enum records whether a source face produced triangles or was malformed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TriangulatedFaceAppendResult {
	Appended,
	CulledDegenerate,
}

/// The `FbxCulledPolygonCounts` struct accumulates concise import diagnostics without logging once per malformed face.
#[derive(Default)]
struct FbxCulledPolygonCounts {
	triangles: usize,
	quads: usize,
	polygons: usize,
}

impl FbxCulledPolygonCounts {
	/// Records one source polygon by its authored corner count for the final import summary.
	fn record(&mut self, corner_count: u32) {
		match corner_count {
			3 => self.triangles += 1,
			4 => self.quads += 1,
			_ => self.polygons += 1,
		}
	}

	/// Adds the malformed geometry summary to the requested resource's trace.
	fn trace(&self, context: BakeContext<'_>) {
		if self.triangles + self.quads + self.polygons == 0 {
			return;
		}
		context.info(format_args!(
			"Culled degenerate FBX geometry: {} triangle(s), {} quad(s), and {} other polygon(s). The most likely cause is repeated or collinear vertex positions, which produce zero-area triangles and undefined normal data.",
			self.triangles,
			self.quads,
			self.polygons,
		));
	}
}

/// Appends a triangulated face into caller-owned scratch and corner storage.
fn append_triangulated_face<A: Allocator>(
	mesh: &ufbx::Mesh,
	face: ufbx::Face,
	scratch: &mut [u32],
	corners: &mut Vec<u32, A>,
) -> Result<TriangulatedFaceAppendResult, FbxImportError> {
	let triangle_count = mesh.triangulate_face(scratch, face) as usize;
	let index_count = triangle_count.saturating_mul(3);
	if index_count > scratch.len() {
		return Err(FbxImportError::TriangulationOverflow);
	}
	let triangles = &scratch[..index_count];
	// Retained triangles may share malformed corner normals with a degenerate sibling, so discard the source polygon as a unit.
	for triangle in triangles.chunks_exact(3) {
		if is_degenerate_fbx_triangle(mesh, triangle)? {
			return Ok(TriangulatedFaceAppendResult::CulledDegenerate);
		}
	}
	corners.extend_from_slice(triangles);
	Ok(TriangulatedFaceAppendResult::Appended)
}

/// Rejects zero-area triangles before their undefined shading directions reach vertex attribute import.
fn is_degenerate_fbx_triangle(mesh: &ufbx::Mesh, triangle: &[u32]) -> Result<bool, FbxImportError> {
	let mut positions = [ufbx::Vec3::default(); 3];
	for (position, &corner) in positions.iter_mut().zip(triangle) {
		let position_index = mesh
			.vertex_position
			.indices
			.get(corner as usize)
			.ok_or(FbxImportError::InvalidCornerIndex)?;
		*position = *mesh
			.vertex_position
			.values
			.get(*position_index as usize)
			.ok_or(FbxImportError::InvalidCornerIndex)?;
	}

	// Authored zero-area faces are already degenerate in mesh-local space, so avoid repeated per-instance transforms here.
	let first_edge = [
		positions[1].x - positions[0].x,
		positions[1].y - positions[0].y,
		positions[1].z - positions[0].z,
	];
	let second_edge = [
		positions[2].x - positions[0].x,
		positions[2].y - positions[0].y,
		positions[2].z - positions[0].z,
	];
	let area = [
		first_edge[1] * second_edge[2] - first_edge[2] * second_edge[1],
		first_edge[2] * second_edge[0] - first_edge[0] * second_edge[2],
		first_edge[0] * second_edge[1] - first_edge[1] * second_edge[0],
	];
	Ok(area == [0.0; 3])
}

/// The `RemappedCorners` struct carries one u16-compatible primitive's source-corner lookup and local indices.
struct RemappedCorners<'a> {
	source_corners: Vec<u32, &'a dyn Allocator>,
	indices: Vec<u32, &'a dyn Allocator>,
}

/// Splits and remaps corner-indexed triangles so every processed primitive remains representable by the engine's u16 index streams.
fn remap_triangle_corners<'a>(
	source_corner_count: usize,
	corners: &[u32],
	remap: &mut [u32],
	allocator: &'a dyn Allocator,
) -> Result<Vec<RemappedCorners<'a>, &'a dyn Allocator>, FbxImportError> {
	if !corners.len().is_multiple_of(3) {
		return Err(FbxImportError::InvalidTriangleCount);
	}
	if remap.len() != source_corner_count {
		return Err(FbxImportError::InvalidCornerIndex);
	}

	let unique_corner_capacity = source_corner_count.min(corners.len()).min(MAX_PRIMITIVE_VERTICES);
	let index_capacity = if source_corner_count <= MAX_PRIMITIVE_VERTICES {
		corners.len()
	} else {
		corners.len().min(MAX_PRIMITIVE_VERTICES.saturating_mul(3))
	};
	let batch_capacity = source_corner_count
		.min(corners.len())
		.div_ceil(MAX_PRIMITIVE_VERTICES.saturating_sub(2))
		.max(1);
	let mut source_corners = Vec::with_capacity_in(unique_corner_capacity, allocator);
	let mut indices = Vec::with_capacity_in(index_capacity, allocator);
	let mut batches = Vec::with_capacity_in(batch_capacity, allocator);

	for triangle in corners.chunks_exact(3) {
		let mut new_corners = 0usize;
		for &corner in triangle {
			let corner = corner as usize;
			if corner >= source_corner_count {
				return Err(FbxImportError::InvalidCornerIndex);
			}
			if remap[corner] == u32::MAX {
				new_corners += 1;
			}
		}

		if !indices.is_empty() && source_corners.len() + new_corners > MAX_PRIMITIVE_VERTICES {
			for &corner in &source_corners {
				remap[corner as usize] = u32::MAX;
			}
			batches.push(RemappedCorners {
				source_corners: std::mem::replace(
					&mut source_corners,
					Vec::with_capacity_in(unique_corner_capacity, allocator),
				),
				indices: std::mem::replace(&mut indices, Vec::with_capacity_in(index_capacity, allocator)),
			});
		}

		for &corner in triangle {
			let slot = &mut remap[corner as usize];
			if *slot == u32::MAX {
				*slot = source_corners.len() as u32;
				source_corners.push(corner);
			}
			indices.push(*slot);
		}
	}

	if !indices.is_empty() {
		for &corner in &source_corners {
			remap[corner as usize] = u32::MAX;
		}
		batches.push(RemappedCorners { source_corners, indices });
	}
	Ok(batches)
}

/// Extracts one remapped FBX primitive while respecting independent per-corner attribute indices.
fn import_fbx_primitive<'a>(
	context: &FbxMeshImportContext<'_>,
	material: ReferenceModel<VariantModel>,
	batch: RemappedCorners<'a>,
	layout: &mut Vec<VertexComponent, &'a dyn Allocator>,
	layout_semantics: &mut VertexAttributeMask,
	allocator: &'a dyn Allocator,
) -> Result<OwnedMeshPrimitive<&'a dyn Allocator>, FbxImportError> {
	if batch.source_corners.is_empty() {
		return Err(FbxImportError::EmptyPrimitive);
	}
	let mesh = context.mesh;
	let mut positions = Vec::with_capacity_in(batch.source_corners.len(), allocator);
	let mut minimum = [f32::INFINITY; 3];
	let mut maximum = [f32::NEG_INFINITY; 3];
	let mut normals = context
		.normal_matrix
		.is_some()
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut tangents = context
		.source_attributes
		.contains(VertexSemantics::Tangent)
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut bitangents = context
		.source_attributes
		.contains(VertexSemantics::BiTangent)
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	// Visibility rendering requires a UV stream even for untextured materials, so absent FBX UVs use the origin.
	let mut uvs = Vec::with_capacity_in(batch.source_corners.len(), allocator);
	let mut colors = context
		.source_attributes
		.contains(VertexSemantics::Color)
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut joints = context
		.skin
		.map(|_| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut weights = context
		.skin
		.map(|_| Vec::with_capacity_in(batch.source_corners.len(), allocator));

	for &source_corner in &batch.source_corners {
		let corner = source_corner as usize;
		let position = ufbx::transform_position(&context.node.geometry_to_world, mesh.vertex_position[corner]);
		let position = vec3_to_f32(position, "mesh position")?;
		for axis in 0..3 {
			minimum[axis] = minimum[axis].min(position[axis]);
			maximum[axis] = maximum[axis].max(position[axis]);
		}
		positions.push(position);

		// Build a world-space orthonormal tangent frame so non-uniform instance scales do not skew shading inputs.
		let normal = context
			.normal_matrix
			.as_ref()
			.map(|normal_matrix| normalized_direction(normal_matrix, mesh.vertex_normal[corner]))
			.transpose()?;
		let transformed_bitangent = context
			.source_attributes
			.contains(VertexSemantics::BiTangent)
			.then(|| normalized_direction(&context.node.geometry_to_world, mesh.vertex_bitangent[corner]))
			.transpose()?;
		let tangent = context
			.source_attributes
			.contains(VertexSemantics::Tangent)
			.then(|| normalized_direction(&context.node.geometry_to_world, mesh.vertex_tangent[corner]))
			.transpose()?
			.map(|tangent| match normal {
				Some(normal) => orthogonalized_direction(tangent, normal),
				None => Ok(tangent),
			})
			.transpose()?;

		if let Some(values) = normals.as_mut() {
			values.push(normal.expect("normal output exists only when the FBX normal attribute exists"));
		}
		if let Some(values) = tangents.as_mut() {
			let tangent = tangent.expect("tangent output exists only when the FBX tangent attribute exists");
			let handedness = match (normal, transformed_bitangent) {
				(Some(normal), Some(bitangent)) => tangent_handedness(normal, tangent, bitangent),
				_ => 1.0,
			};
			values.push([tangent[0], tangent[1], tangent[2], handedness]);
		}
		if let Some(values) = bitangents.as_mut() {
			let bitangent = match (normal, tangent, transformed_bitangent) {
				(Some(normal), Some(tangent), Some(bitangent)) => {
					let handedness = tangent_handedness(normal, tangent, bitangent);
					scale_vec3(cross_vec3(normal, tangent), handedness)
				}
				(Some(normal), None, Some(bitangent)) => orthogonalized_direction(bitangent, normal)?,
				(_, _, Some(bitangent)) => bitangent,
				(_, _, None) => unreachable!("bitangent output exists only when the FBX bitangent attribute exists"),
			};
			values.push(bitangent);
		}
		if context.source_attributes.contains(VertexSemantics::UV) {
			let uv = mesh.vertex_uv[corner];
			uvs.push([finite_f32(uv.x, "mesh UV")?, finite_f32(uv.y, "mesh UV")?]);
		} else {
			uvs.push([0.0, 0.0]);
		}
		if let Some(values) = colors.as_mut() {
			let color = mesh.vertex_color[corner];
			values.push([
				finite_f32(color.x, "mesh color")?,
				finite_f32(color.y, "mesh color")?,
				finite_f32(color.z, "mesh color")?,
				finite_f32(color.w, "mesh color")?,
			]);
		}
		if let (Some(skin), Some(joints), Some(weights)) = (context.skin, joints.as_mut(), weights.as_mut()) {
			let logical_vertex = *mesh.vertex_indices.get(corner).ok_or(FbxImportError::InvalidCornerIndex)? as usize;
			let (vertex_joints, vertex_weights) = skin_weights(skin, logical_vertex, context.fallback_joint)?;
			joints.push(vertex_joints);
			weights.push(vertex_weights);
		}
	}

	let bounds = [minimum, maximum];
	let mut triangle_indices = batch.indices;
	if context.mirrored {
		// Preserve the configured global front face when an authored instance mirrors its flattened geometry.
		for triangle in triangle_indices.chunks_exact_mut(3) {
			triangle.swap(1, 2);
		}
	}
	let mut primitive = OwnedMeshPrimitive::new_in(material, bounds, triangle_indices, allocator);
	primitive.set_transform_node(context.transform_node);
	primitive.set_skin(context.skin_index);
	add_mesh_attribute(
		&mut primitive,
		layout,
		layout_semantics,
		VertexSemantics::Position,
		"vec3f",
		OwnedMeshAttributeData::F32x3(positions),
	);
	if let Some(values) = normals {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Normal,
			"vec3f",
			OwnedMeshAttributeData::F32x3(values),
		);
	}
	if let Some(values) = tangents {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Tangent,
			"vec4f",
			OwnedMeshAttributeData::F32x4(values),
		);
	}
	if let Some(values) = bitangents {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::BiTangent,
			"vec3f",
			OwnedMeshAttributeData::F32x3(values),
		);
	}
	add_mesh_attribute(
		&mut primitive,
		layout,
		layout_semantics,
		VertexSemantics::UV,
		"vec2f",
		OwnedMeshAttributeData::F32x2(uvs),
	);
	if let Some(values) = colors {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Color,
			"vec4f",
			OwnedMeshAttributeData::F32x4(values),
		);
	}
	if let Some(values) = joints {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Joints,
			"vec4u16",
			OwnedMeshAttributeData::U16x4(values),
		);
	}
	if let Some(values) = weights {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Weights,
			"vec4f",
			OwnedMeshAttributeData::F32x4(values),
		);
	}
	Ok(primitive)
}

/// Adds attribute payload and records its shared layout declaration on first use.
fn add_mesh_attribute<'a>(
	primitive: &mut OwnedMeshPrimitive<&'a dyn Allocator>,
	layout: &mut Vec<VertexComponent, &'a dyn Allocator>,
	layout_semantics: &mut VertexAttributeMask,
	semantic: VertexSemantics,
	format: &str,
	data: OwnedMeshAttributeData<&'a dyn Allocator>,
) {
	if layout_semantics.insert(semantic) {
		layout.push(VertexComponent {
			semantic,
			format: format.to_string(),
			channel: 0,
		});
	}
	primitive.add_attribute(OwnedMeshAttribute::new(semantic, 0, data));
}

/// Selects and normalizes the four strongest influences, routing unweighted vertices to the animated mesh-node fallback.
fn skin_weights(
	skin: &ufbx::SkinDeformer,
	logical_vertex: usize,
	fallback_joint: Option<u16>,
) -> Result<([u16; 4], [f32; 4]), FbxImportError> {
	let influences = skin_influences(skin, logical_vertex)?;
	let mut joints = [0u16; 4];
	let mut weights = [0.0f32; 4];
	let mut total = 0.0f64;

	// `clean_skin_weights` makes each ufbx influence range strongest-first, so truncation does not
	// need a transient sorting buffer and remains deterministic for the fixed-width GPU stream.
	for (index, influence) in influences.iter().take(4).enumerate() {
		if influence.cluster_index as usize >= skin.clusters.len() {
			return Err(FbxImportError::InvalidSkinCluster);
		}
		joints[index] = influence.cluster_index as u16;
		weights[index] = finite_f32(influence.weight, "skin weight")?.max(0.0);
		total += weights[index] as f64;
	}
	if total > 0.0 {
		for weight in &mut weights {
			*weight = (*weight as f64 / total) as f32;
		}
	} else {
		joints[0] = fallback_joint.ok_or(FbxImportError::MissingFallbackJoint)?;
		weights[0] = 1.0;
	}
	Ok((joints, weights))
}

/// Transforms and normalizes a direction while rejecting degenerate authored values.
fn normalized_direction(matrix: &ufbx::Matrix, direction: ufbx::Vec3) -> Result<[f32; 3], FbxImportError> {
	let direction = ufbx::transform_direction(matrix, direction);
	normalize_vec3(vec3_to_f32(direction, "mesh direction")?)
}

/// Removes the normal component from a transformed tangent-space direction and normalizes the result.
fn orthogonalized_direction(direction: [f32; 3], normal: [f32; 3]) -> Result<[f32; 3], FbxImportError> {
	let alignment = dot_vec3(direction, normal);
	normalize_vec3([
		direction[0] - normal[0] * alignment,
		direction[1] - normal[1] * alignment,
		direction[2] - normal[2] * alignment,
	])
}

/// Normalizes an imported vector without allowing zero-length or non-finite shading data.
fn normalize_vec3(mut direction: [f32; 3]) -> Result<[f32; 3], FbxImportError> {
	let length_squared = direction.iter().map(|component| component * component).sum::<f32>();
	if !length_squared.is_finite() || length_squared <= f32::MIN_POSITIVE {
		return Err(FbxImportError::ZeroDirection);
	}
	let inverse_length = length_squared.sqrt().recip();
	for component in &mut direction {
		*component *= inverse_length;
	}
	Ok(direction)
}

/// Computes tangent-space orientation after the node's geometry transform has been applied.
fn tangent_handedness(normal: [f32; 3], tangent: [f32; 3], bitangent: [f32; 3]) -> f32 {
	let alignment = dot_vec3(cross_vec3(normal, tangent), bitangent);
	if alignment < 0.0 {
		-1.0
	} else {
		1.0
	}
}

/// Computes the dot product used by tangent-frame orthonormalization.
fn dot_vec3(left: [f32; 3], right: [f32; 3]) -> f32 {
	left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

/// Computes the cross product used to reconstruct an orthonormal bitangent.
fn cross_vec3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

/// Applies tangent-space handedness without allocating an intermediate vector.
fn scale_vec3(value: [f32; 3], scale: f32) -> [f32; 3] {
	[value[0] * scale, value[1] * scale, value[2] * scale]
}

/// Converts ufbx's double-precision vectors to the engine's finite single-precision representation.
fn vec3_to_f32(value: ufbx::Vec3, context: &'static str) -> Result<[f32; 3], FbxImportError> {
	Ok([
		finite_f32(value.x, context)?,
		finite_f32(value.y, context)?,
		finite_f32(value.z, context)?,
	])
}

/// Converts ufbx's x/y/z/w quaternion layout to finite single-precision components.
fn quat_to_f32(value: ufbx::Quat, context: &'static str) -> Result<[f32; 4], FbxImportError> {
	Ok([
		finite_f32(value.x, context)?,
		finite_f32(value.y, context)?,
		finite_f32(value.z, context)?,
		finite_f32(value.w, context)?,
	])
}

/// Converts imported numeric data to f32 while retaining an error context for malformed files.
fn finite_f32(value: f64, context: &'static str) -> Result<f32, FbxImportError> {
	if value.is_finite() && value >= f32::MIN as f64 && value <= f32::MAX as f64 {
		Ok(value as f32)
	} else {
		Err(FbxImportError::NonFinite(context))
	}
}

/// Copies authored names only when they contain a useful resource label.
fn non_empty_name(name: &ufbx::String) -> Option<String> {
	(!name.is_empty()).then(|| name.as_ref().to_string())
}

#[derive(Debug, PartialEq, Eq)]
/// The `FbxImportError` enum identifies malformed or unsupported FBX content at the importer boundary.
enum FbxImportError {
	Parse(String),
	AnimationBake(String),
	AnimationNotFound(String),
	UnsupportedFragment(String),
	NoMesh,
	MissingMaterial,
	InvalidFaceIndex,
	InvalidCornerIndex,
	InvalidTriangleCount,
	TriangulationOverflow,
	EmptyPrimitive,
	InvalidSkinVertex,
	InvalidSkinCluster,
	MissingSkinBone,
	MissingFallbackJoint,
	TooManyJoints,
	TooManySkinBindings,
	MultipleSkinDeformers,
	UnsupportedDualQuaternionSkinning,
	NonInvertibleSkinTransform,
	NonInvertibleAnimatedMeshTransform,
	InvalidSkeletonNode,
	DuplicateSkeletonNode,
	IncompleteSkeleton,
	TooManySkeletonNodes,
	ZeroDirection,
	NonFinite(&'static str),
}

impl fmt::Display for FbxImportError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Parse(description) => write!(
				formatter,
				"FBX parsing failed. The most likely cause is malformed or unsupported FBX data: {description}"
			),
			Self::AnimationBake(description) => write!(
				formatter,
				"FBX animation baking failed. The most likely cause is malformed animation curves or unsupported layer data: {description}"
			),
			Self::AnimationNotFound(description) => write!(
				formatter,
				"FBX animation was not found. The most likely cause is an incorrect animation fragment: {description}"
			),
			Self::UnsupportedFragment(fragment) => write!(
				formatter,
				"FBX fragment is unsupported. The most likely cause is that '{fragment}' does not use `skeleton`, `animation`, or `animations/<index-or-name>`."
			),
			Self::NoMesh => write!(
				formatter,
				"FBX mesh is empty. The most likely cause is that the file contains no polygon mesh instances."
			),
			Self::MissingMaterial => write!(
				formatter,
				"FBX material resolution failed. The most likely cause is inconsistent material metadata in the imported scene."
			),
			Self::InvalidFaceIndex => write!(
				formatter,
				"FBX face index is invalid. The most likely cause is a malformed material part referencing a missing face."
			),
			Self::InvalidCornerIndex => write!(
				formatter,
				"FBX vertex-corner index is invalid. The most likely cause is malformed polygon topology."
			),
			Self::InvalidTriangleCount => write!(
				formatter,
				"FBX triangle index count is invalid. The most likely cause is incomplete triangulation output."
			),
			Self::TriangulationOverflow => write!(
				formatter,
				"FBX triangulation exceeded its scratch buffer. The most likely cause is inconsistent maximum-face metadata."
			),
			Self::EmptyPrimitive => write!(
				formatter,
				"FBX primitive has no vertices. The most likely cause is an empty or degenerate material part."
			),
			Self::InvalidSkinVertex => write!(
				formatter,
				"FBX skin vertex is invalid. The most likely cause is skin weights that do not match the mesh control vertices."
			),
			Self::InvalidSkinCluster => write!(
				formatter,
				"FBX skin cluster is invalid. The most likely cause is a weight referencing a missing joint palette entry."
			),
			Self::MissingSkinBone => write!(
				formatter,
				"FBX skin cluster has no bone. The most likely cause is a broken cluster-to-node connection."
			),
			Self::MissingFallbackJoint => write!(
				formatter,
				"FBX fallback joint is missing. The most likely cause is an unweighted vertex without its required mesh-node palette entry."
			),
			Self::TooManyJoints => write!(
				formatter,
				"FBX skin has too many joints. The most likely cause is a joint palette larger than the engine's u16 joint stream."
			),
			Self::TooManySkinBindings => write!(
				formatter,
				"FBX has too many skin bindings. The most likely cause is more skinned mesh instances than the resource format can index."
			),
			Self::MultipleSkinDeformers => write!(
				formatter,
				"FBX mesh has multiple skin deformers. The most likely cause is layered skinning that cannot be represented by one matrix palette."
			),
			Self::UnsupportedDualQuaternionSkinning => write!(
				formatter,
				"FBX dual-quaternion skinning is unsupported. The most likely cause is a dual-quaternion or blended skin deformer authored on the mesh."
			),
			Self::NonInvertibleSkinTransform => write!(
				formatter,
				"FBX skin transform is not invertible. The most likely cause is a zero-scale skinned mesh instance."
			),
			Self::NonInvertibleAnimatedMeshTransform => write!(
				formatter,
				"FBX animated mesh transform is not invertible. The most likely cause is a zero bind scale that cannot be recovered after flattening geometry."
			),
			Self::InvalidSkeletonNode => write!(
				formatter,
				"FBX skeleton node is invalid. The most likely cause is a node ID outside the imported scene hierarchy."
			),
			Self::DuplicateSkeletonNode => write!(
				formatter,
				"FBX skeleton node is duplicated. The most likely cause is an inconsistent node hierarchy containing the same child twice."
			),
			Self::IncompleteSkeleton => write!(
				formatter,
				"FBX skeleton hierarchy is incomplete. The most likely cause is a scene node disconnected from the imported root."
			),
			Self::TooManySkeletonNodes => write!(
				formatter,
				"FBX skeleton has too many nodes. The most likely cause is a hierarchy larger than the resource's u32 node indices."
			),
			Self::ZeroDirection => write!(
				formatter,
				"FBX direction vector is zero. The most likely cause is malformed normal or tangent data."
			),
			Self::NonFinite(context) => write!(
				formatter,
				"FBX numeric value is invalid. The most likely cause is a non-finite or out-of-range {context}."
			),
		}
	}
}

impl std::error::Error for FbxImportError {}

#[cfg(test)]
mod tests {
	use std::{
		alloc::{Allocator, Global},
		collections::HashMap,
	};

	use super::{
		fbx_brdf_material, finite_material_component, finite_material_product, import_fbx_animation, import_fbx_meshes,
		import_fbx_skeleton, import_fbx_skin_binding, load_fbx_scene, matrix_to_columns, remap_triangle_corners,
		select_fbx_skin, select_unfragmented_fbx_resource, skin_weights, FBXAssetHandler, FbxCulledPolygonCounts,
		FbxImportError, MaterialKey, ResolvedFbxMaterials,
	};
	use crate::{
		asset::{
			asset_handler::AssetHandler, asset_manager::AssetManager, bema_asset_handler::tests::MinimalTestShaderGenerator,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend, ContainerDefaultResource,
		},
		pbr::{BrdfAlphaMode, BrdfMaterialDescription, BrdfNode, BrdfValue},
		processors::mesh_processor::{
			MeshAttributeData, MeshIndexData, MeshPrimitiveSource, MeshProcessor, MeshSource, OwnedMeshSource,
			TriangleFrontFaceWinding,
		},
		r#async,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::{
			animation::{AnimationModel, QuaternionCurve, Vector3Curve},
			material::{MaterialModel, VariantModel},
			mesh::MeshModel,
			skeleton::{SkeletonModel, SkinJoint},
		},
		types::{AlphaMode, IndexStreamTypes, VertexSemantics},
		ReferenceModel,
	};
	#[cfg(debug_assertions)]
	use crate::{
		asset::{asset_handler::BakeContext, asset_handler::LoadErrors, ResourceId, ResourceTraceLevel},
		ProcessedAsset,
	};

	const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("test_data/triangle_move_ascii.fbx");
	const ANIMATION_ONLY_FBX: &[u8] = include_bytes!("test_data/animation_only_ascii.fbx");
	const DEGENERATE_QUAD_FBX: &[u8] = include_bytes!("test_data/degenerate_quad_ascii.fbx");
	const MATERIAL_FACTORS_FBX: &[u8] = include_bytes!("test_data/material_factors_ascii.fbx");
	const SKINNED_TRIANGLE_FBX: &[u8] = include_bytes!("test_data/skinned_triangle_ascii.fbx");

	/// Imports a fixture while discarding diagnostic counts that are not relevant to the focused assertion.
	fn import_test_fbx_meshes<'a>(
		scene: &ufbx::Scene,
		materials: &ResolvedFbxMaterials,
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		source_to_skeleton: &[u32],
		allocator: &'a dyn Allocator,
	) -> Result<OwnedMeshSource<&'a dyn Allocator>, FbxImportError> {
		let mut culled_polygons = FbxCulledPolygonCounts::default();
		import_fbx_meshes(
			scene,
			materials,
			skeleton,
			source_to_skeleton,
			allocator,
			&mut culled_polygons,
		)
	}

	/// The `TestVariantAssetHandler` struct supplies a material override without invoking a platform shader compiler.
	#[cfg(debug_assertions)]
	struct TestVariantAssetHandler;

	#[cfg(debug_assertions)]
	impl AssetHandler for TestVariantAssetHandler {
		fn can_handle(&self, resource_type: &str) -> bool {
			resource_type == "variant"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			context.store_primary(
				ProcessedAsset::new(
					id,
					VariantModel {
						material: ReferenceModel::<MaterialModel>::new_serialized(
							"materials/test.material",
							0,
							0,
							Vec::new(),
							None,
						),
						variables: Vec::new(),
						alpha_mode: AlphaMode::Opaque,
					},
				),
				&[],
			)
		}
	}

	#[test]
	fn recognizes_fbx_and_exposes_consistent_default_winding() {
		let handler = FBXAssetHandler::new();

		assert!(handler.can_handle("fbx"));
		assert!(handler.can_handle("FBX"));
		assert!(!handler.can_handle("glb"));
		assert_eq!(handler.triangle_front_face_winding(), TriangleFrontFaceWinding::Clockwise);
	}

	#[test]
	fn unfragmented_fbx_with_geometry_remains_mesh_first() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").unwrap();
		assert_eq!(
			select_unfragmented_fbx_resource(&scene, None),
			Ok(ContainerDefaultResource::Mesh)
		);
	}

	#[test]
	fn imports_triangulated_mesh_attributes_and_meter_scaled_bounds() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").expect("fixture FBX should parse");
		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};
		let source = import_test_fbx_meshes(&scene, &materials, None, &[], &Global).expect("fixture mesh should import");
		let processed = MeshProcessor::new().process(&source).expect("fixture mesh should process");

		assert!(processed.mesh.skeleton.is_none());
		assert!(processed.mesh.skins.is_empty());
		assert_eq!(processed.mesh.primitives.len(), 1);
		assert_eq!(processed.mesh.primitives[0].vertex_count, 3);
		assert!(processed
			.mesh
			.vertex_components
			.iter()
			.any(|component| component.semantic == VertexSemantics::Position));
		assert!(processed
			.mesh
			.vertex_components
			.iter()
			.any(|component| component.semantic == VertexSemantics::Normal));
		assert!(processed
			.mesh
			.vertex_components
			.iter()
			.any(|component| component.semantic == VertexSemantics::UV));
		let bounds = processed.mesh.primitives[0].bounding_box;
		assert_eq!(bounds[0], [0.0, 0.0, 0.0]);
		assert!((bounds[1][0] - 0.01).abs() < 1.0e-6);
		assert!((bounds[1][1] - 0.01).abs() < 1.0e-6);
		assert_eq!(bounds[1][2], 0.0);
	}

	#[test]
	fn discards_degenerate_polygons_without_rejecting_valid_mesh_geometry() {
		let scene = load_fbx_scene(DEGENERATE_QUAD_FBX, "degenerate_quad.fbx").expect("fixture FBX should parse");
		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};
		let source = import_test_fbx_meshes(&scene, &materials, None, &[], &Global)
			.expect("degenerate polygons should be discarded without rejecting valid geometry");
		let primitive = source.primitive(0).expect("valid triangle should remain");

		let Some(MeshAttributeData::F32x3(positions)) = primitive.attribute(VertexSemantics::Position, 0) else {
			panic!("FBX fixture should contain f32 position data");
		};
		let Some(MeshIndexData::U32(indices)) = primitive.indices(IndexStreamTypes::Triangles) else {
			panic!("FBX fixture should contain triangle indices");
		};
		assert_eq!(positions.len(), 3);
		assert_eq!(indices.len(), 3);
	}

	#[test]
	fn normalizes_handedness_and_mirrored_instance_winding_before_mesh_processing() {
		let fixture = std::str::from_utf8(MATERIAL_FACTORS_FBX).expect("material fixture should be UTF-8");
		let right_handed = fixture.replace(
			"P: \"FrontAxisSign\", \"int\", \"Integer\", \"\",-1",
			"P: \"FrontAxisSign\", \"int\", \"Integer\", \"\",1",
		);
		let mirrored = fixture.replace(
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",1,1,1",
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",-1,1,1",
		);
		assert_ne!(right_handed, fixture);
		assert_ne!(mirrored, fixture);

		let base_scene = load_fbx_scene(MATERIAL_FACTORS_FBX, "base.fbx").expect("base fixture should parse");
		let right_handed_scene =
			load_fbx_scene(right_handed.as_bytes(), "right_handed.fbx").expect("right-handed fixture should parse");
		let mirrored_scene = load_fbx_scene(mirrored.as_bytes(), "mirrored.fbx").expect("mirrored fixture should parse");

		assert!(!right_handed_scene.meshes[0].reversed_winding);
		let base_area = first_clockwise_triangle_area(&base_scene);
		let right_handed_area = first_clockwise_triangle_area(&right_handed_scene);
		let mirrored_area = first_clockwise_triangle_area(&mirrored_scene);
		assert!(base_area.abs() > f32::EPSILON);
		assert_eq!(right_handed_area.signum(), base_area.signum());
		assert_eq!(mirrored_area.signum(), base_area.signum());
	}

	#[test]
	fn imports_named_and_indexed_animation_fragments_with_zero_based_seconds() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").expect("fixture FBX should parse");
		let imported_skeleton = import_fbx_skeleton(&scene).expect("fixture skeleton should import");
		let skeleton = test_skeleton(&imported_skeleton.model);
		let named = import_fbx_animation(
			&scene,
			"animations/MoveX",
			skeleton.clone(),
			&imported_skeleton.source_to_skeleton,
		)
		.expect("named take should import");
		let indexed = import_fbx_animation(
			&scene,
			"animations/0",
			skeleton.clone(),
			&imported_skeleton.source_to_skeleton,
		)
		.expect("indexed take should import");
		let default = import_fbx_animation(&scene, "animation", skeleton.clone(), &imported_skeleton.source_to_skeleton)
			.expect("default take should import");

		assert_eq!(named.name.as_deref(), Some("MoveX"));
		assert_eq!(indexed.name, named.name);
		assert_eq!(default.name, named.name);
		assert!((named.duration - 1.0).abs() < f32::EPSILON);
		let translation_track = named
			.tracks
			.iter()
			.find(|track| track.translation.is_some())
			.expect("animated node should have a translation track");
		let Some(Vector3Curve::Linear { times, values }) = &translation_track.translation else {
			panic!("FBX translation track has the wrong curve type. The most likely cause is a track conversion regression.");
		};
		assert_eq!(times.first().copied(), Some(0.0));
		assert_eq!(times.last().copied(), Some(1.0));
		assert!((values.last().unwrap()[0] - 0.02).abs() < 1.0e-6);
		assert!(matches!(
			import_fbx_animation(&scene, "mesh", skeleton, &imported_skeleton.source_to_skeleton,),
			Err(FbxImportError::UnsupportedFragment(_))
		));
	}

	#[test]
	fn rejects_singular_bind_transforms_for_node_driven_rigid_geometry() {
		let fixture = std::str::from_utf8(TRIANGLE_MOVE_FBX).expect("animation fixture should be UTF-8");
		let zero_scale = fixture.replace(
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",1,1,1",
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",0,1,1",
		);
		assert_ne!(zero_scale, fixture);

		let scene =
			load_fbx_scene(zero_scale.as_bytes(), "zero_scale_animation.fbx").expect("zero-scale FBX fixture should parse");
		let imported_skeleton = import_fbx_skeleton(&scene).expect("fixture hierarchy should import");
		let skeleton = test_skeleton(&imported_skeleton.model);
		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};

		assert!(matches!(
			import_test_fbx_meshes(
				&scene,
				&materials,
				Some(skeleton),
				&imported_skeleton.source_to_skeleton,
				&Global,
			),
			Err(FbxImportError::NonInvertibleAnimatedMeshTransform)
		));
	}

	#[test]
	fn imports_skinned_hierarchy_binding_weights_and_remapped_rotation_track() {
		let scene = load_fbx_scene(SKINNED_TRIANGLE_FBX, "skinned_triangle.fbx").expect("skinned fixture FBX should parse");
		let imported_skeleton = import_fbx_skeleton(&scene).expect("skinned hierarchy should import");
		let root = scene
			.nodes
			.iter()
			.find(|node| node.element.name.as_ref() == "RootJoint")
			.expect("fixture should contain RootJoint");
		let child = scene
			.nodes
			.iter()
			.find(|node| node.element.name.as_ref() == "ChildJoint")
			.expect("fixture should contain ChildJoint");
		let root_index = imported_skeleton.source_to_skeleton[root.element.typed_id as usize];
		let child_index = imported_skeleton.source_to_skeleton[child.element.typed_id as usize];
		assert!(root_index < child_index);
		assert_eq!(imported_skeleton.model.nodes[child_index as usize].parent, Some(root_index));

		let mesh_node = scene
			.nodes
			.iter()
			.find(|node| node.mesh.is_some())
			.expect("fixture should contain a mesh node");
		let mesh_node_index = imported_skeleton.source_to_skeleton[mesh_node.element.typed_id as usize];
		let skin = select_fbx_skin(mesh_node.mesh.as_ref().unwrap())
			.expect("fixture skin should be supported")
			.expect("fixture mesh should be skinned");
		let (binding, fallback_joint) = import_fbx_skin_binding(mesh_node, skin, &imported_skeleton.source_to_skeleton)
			.expect("fixture skin binding should import");
		assert_eq!(fallback_joint, None);
		assert_eq!(
			binding.entries.iter().map(|entry| entry.joint).collect::<Vec<_>>(),
			[SkinJoint::Node(root_index), SkinJoint::Node(child_index)]
		);
		assert_eq!(binding.len(), 2);

		// The palette must match ufbx's evaluated clusters after expressing them in
		// the flattened vertex basis used by the imported mesh.
		let mut globals = vec![crate::resources::skeleton::identity_matrix4_columns(); imported_skeleton.model.nodes.len()];
		for node in &scene.nodes {
			let mapped = imported_skeleton.source_to_skeleton[node.element.typed_id as usize] as usize;
			globals[mapped] = matrix_to_columns(&node.node_to_world).expect("fixture global matrix should be finite");
		}
		let mut palette = vec![crate::resources::skeleton::identity_matrix4_columns(); binding.len()];
		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("fixture palette should be complete");
		let flattened_inverse = ufbx::matrix_invert(&mesh_node.geometry_to_world);
		for (matrix, cluster) in palette.into_iter().zip(&skin.clusters) {
			let expected = ufbx::matrix_mul(&cluster.geometry_to_world, &flattened_inverse);
			assert_matrix_close(
				matrix,
				matrix_to_columns(&expected).expect("expected fixture palette matrix should be finite"),
			);
		}

		let (joints, weights) = skin_weights(skin, 1, fallback_joint).expect("mixed fixture weights should import");
		assert_eq!(&joints[..2], &[1, 0]);
		assert!((weights[0] - 0.75).abs() < 1.0e-6);
		assert!((weights[1] - 0.25).abs() < 1.0e-6);

		let skeleton = test_skeleton(&imported_skeleton.model);
		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};
		let source = import_test_fbx_meshes(
			&scene,
			&materials,
			Some(skeleton.clone()),
			&imported_skeleton.source_to_skeleton,
			&Global,
		)
		.expect("skinned fixture mesh should import");
		let processed = MeshProcessor::new()
			.process(&source)
			.expect("skinned fixture mesh should process");
		assert_eq!(processed.mesh.skeleton.as_ref().map(|value| value.id()), Some(skeleton.id()));
		assert_eq!(processed.mesh.skins.len(), 1);
		assert_eq!(processed.mesh.primitives[0].transform_node, Some(mesh_node_index));
		assert_eq!(processed.mesh.primitives[0].skin, Some(0));

		let animation = import_fbx_animation(&scene, "animations/Bend", skeleton, &imported_skeleton.source_to_skeleton)
			.expect("skinned fixture animation should import");
		let track = animation
			.tracks
			.iter()
			.find(|track| track.node == child_index)
			.expect("child rotation should target the remapped skeleton node");
		let Some(QuaternionCurve::Linear { times, values }) = &track.rotation else {
			panic!("FBX child rotation should import as a linear quaternion curve");
		};
		assert_eq!(times.first().copied(), Some(0.0));
		assert_eq!(times.last().copied(), Some(1.0));
		assert_ne!(values.first(), values.last());
	}

	#[test]
	fn routes_unweighted_vertices_to_the_animated_mesh_node() {
		let fixture = std::str::from_utf8(SKINNED_TRIANGLE_FBX).expect("skinned fixture should be UTF-8");
		let without_last_weight = fixture.replace(
			"Indexes: *2 {\n            a: 1,2\n        }\n        Weights: *2 {\n            a: 0.75,1\n        }",
			"Indexes: *1 {\n            a: 1\n        }\n        Weights: *1 {\n            a: 0.75\n        }",
		);
		assert_ne!(without_last_weight, fixture);
		let scene = load_fbx_scene(without_last_weight.as_bytes(), "unweighted_triangle.fbx")
			.expect("unweighted fixture variant should parse");
		let imported_skeleton = import_fbx_skeleton(&scene).expect("fixture hierarchy should import");
		let mesh_node = scene
			.nodes
			.iter()
			.find(|node| node.mesh.is_some())
			.expect("fixture should contain a mesh node");
		let skin = select_fbx_skin(mesh_node.mesh.as_ref().unwrap())
			.expect("fixture skin should be supported")
			.expect("fixture mesh should be skinned");
		let (binding, fallback_joint) = import_fbx_skin_binding(mesh_node, skin, &imported_skeleton.source_to_skeleton)
			.expect("unweighted fixture binding should import");
		let fallback_joint = fallback_joint.expect("unweighted vertices require a mesh-node palette entry");
		let mesh_node_index = imported_skeleton.source_to_skeleton[mesh_node.element.typed_id as usize];
		assert_eq!(
			binding.entries[fallback_joint as usize].joint,
			SkinJoint::Node(mesh_node_index)
		);

		let mut globals = vec![crate::resources::skeleton::identity_matrix4_columns(); imported_skeleton.model.nodes.len()];
		for node in &scene.nodes {
			let mapped = imported_skeleton.source_to_skeleton[node.element.typed_id as usize] as usize;
			globals[mapped] = matrix_to_columns(&node.node_to_world).expect("fixture global matrix should be finite");
		}
		let mut palette = vec![crate::resources::skeleton::identity_matrix4_columns(); binding.len()];
		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("fallback palette should be complete");
		assert_matrix_close(
			palette[fallback_joint as usize],
			crate::resources::skeleton::identity_matrix4_columns(),
		);
		// Moving the mesh node after bind must move the fallback palette entry instead of freezing the vertex.
		globals[mesh_node_index as usize][3][0] += 1.0;
		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("animated fallback palette should remain complete");
		assert!((palette[fallback_joint as usize][3][0] - 1.0).abs() < 1.0e-6);

		let (joints, weights) = skin_weights(skin, 2, Some(fallback_joint)).expect("unweighted vertex should import");
		assert_eq!(joints[0], fallback_joint);
		assert_eq!(weights, [1.0, 0.0, 0.0, 0.0]);
	}

	#[test]
	fn rejects_dual_quaternion_and_multiple_skin_deformers_explicitly() {
		let fixture = std::str::from_utf8(SKINNED_TRIANGLE_FBX).expect("skinned fixture should be UTF-8");
		let dual_quaternion = fixture.replace("SkinningType: \"Linear\"", "SkinningType: \"DualQuaternion\"");
		assert_ne!(dual_quaternion, fixture);
		let scene = load_fbx_scene(dual_quaternion.as_bytes(), "dual_quaternion.fbx")
			.expect("dual-quaternion fixture variant should parse");
		let mesh = scene.meshes.first().expect("fixture should contain a mesh");
		assert!(matches!(
			select_fbx_skin(mesh),
			Err(FbxImportError::UnsupportedDualQuaternionSkinning)
		));

		let extra_skin = r#"    Deformer: 1300, "Deformer::ExtraSkin", "Skin" {
        Version: 101
        Link_DeformAcuracy: 50
        SkinningType: "Linear"
    }

"#;
		let multiple = fixture
			.replace(
				"    Deformer: 1301, \"Deformer::TriangleSkin\", \"Skin\" {",
				&format!("{extra_skin}    Deformer: 1301, \"Deformer::TriangleSkin\", \"Skin\" {{"),
			)
			.replace("    C: \"OO\",1301,1001", "    C: \"OO\",1300,1001\n    C: \"OO\",1301,1001");
		assert_ne!(multiple, fixture);
		let scene =
			load_fbx_scene(multiple.as_bytes(), "multiple_skins.fbx").expect("multiple-skin fixture variant should parse");
		let mesh = scene.meshes.first().expect("fixture should contain a mesh");
		assert!(matches!(select_fbx_skin(mesh), Err(FbxImportError::MultipleSkinDeformers)));
	}

	#[test]
	fn broadcasts_scalar_material_factors_and_derives_legacy_opacity() {
		let scene = load_fbx_scene(MATERIAL_FACTORS_FBX, "material_factors.fbx").expect("material fixture should parse");
		let phong = ufbx::find_material(&scene, "FactoredPhong").expect("Phong material should exist");
		let metal_rough = ufbx::find_material(&scene, "FactoredMetalRough").expect("PBR material should exist");

		let phong_brdf = fbx_brdf_material(Some(phong));
		let (base_color, metallic, roughness, emission) = brdf_values(&phong_brdf);
		assert_vec4_close(base_color, [0.2, 0.1, 0.05, 0.8]);
		assert!((metallic - 0.0).abs() < 1.0e-6);
		assert!((roughness - 0.6).abs() < 1.0e-6);
		assert_vec3_close(emission, [0.2, 0.6, 1.0]);
		assert_eq!(phong_brdf.alpha_mode, BrdfAlphaMode::Blend);

		let pbr_brdf = fbx_brdf_material(Some(metal_rough));
		let (base_color, metallic, roughness, emission) = brdf_values(&pbr_brdf);
		assert_vec4_close(base_color, [0.25, 0.5, 0.75, 0.4]);
		assert!((metallic - 0.65).abs() < 1.0e-6);
		assert!((roughness - 0.35).abs() < 1.0e-6);
		assert_vec3_close(emission, [0.05, 0.1, 0.15]);

		let materials = fixture_materials(&scene);
		let source = import_test_fbx_meshes(&scene, &materials, None, &[], &Global).expect("material-part mesh should import");
		let processed = MeshProcessor::new()
			.process(&source)
			.expect("material-part mesh should process");
		let material_ids = processed
			.mesh
			.primitives
			.iter()
			.map(|primitive| primitive.material.id().as_ref().to_string())
			.collect::<Vec<_>>();
		assert_eq!(processed.mesh.primitives.len(), 2);
		assert!(material_ids.iter().any(|id| id.ends_with("FactoredPhong.variant")));
		assert!(material_ids.iter().any(|id| id.ends_with("FactoredMetalRough.variant")));
	}

	#[test]
	fn malformed_fbx_returns_a_parse_error() {
		assert!(matches!(
			load_fbx_scene(b"not an FBX", "broken.fbx"),
			Err(FbxImportError::Parse(_))
		));
	}

	#[test]
	fn reusable_corner_remap_restores_scratch_and_rejects_invalid_indices() {
		let mut remap = vec![u32::MAX; 4];
		let batches =
			remap_triangle_corners(4, &[0, 1, 2, 2, 1, 3], &mut remap, &Global).expect("valid triangles should remap");

		assert_eq!(batches.len(), 1);
		assert_eq!(batches[0].source_corners, vec![0, 1, 2, 3]);
		assert_eq!(batches[0].indices, vec![0, 1, 2, 2, 1, 3]);
		assert!(remap.iter().all(|&slot| slot == u32::MAX));
		assert!(matches!(
			remap_triangle_corners(4, &[0, 1, 4], &mut remap, &Global),
			Err(FbxImportError::InvalidCornerIndex)
		));
	}

	#[test]
	fn material_numeric_conversion_replaces_non_finite_and_overflowing_values() {
		assert_eq!(finite_material_component(f64::MAX, 0.25), 0.25);
		assert_eq!(finite_material_component(f64::NAN, 0.5), 0.5);
		assert_eq!(finite_material_product(f32::MAX, f32::MAX, 0.0), 0.0);
	}

	#[r#async::test]
	async fn asset_manager_bakes_animation_fragment_without_a_shader_generator() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("triangle_move.fbx", TRIANGLE_MOVE_FBX);
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(FBXAssetHandler::new());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx#animations/MoveX", &resource_storage)
			.await
			.expect("FBX animation fragment should bake");

		assert_eq!(animation.class(), "Animation");
		assert_eq!(animation.id().as_ref(), "triangle_move.fbx#animations/MoveX");
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn asset_manager_associates_culled_geometry_info_with_the_baked_fbx_resource() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("degenerate_quad.fbx", DEGENERATE_QUAD_FBX);
		asset_storage.add_file(
			"degenerate_quad.fbx.bead",
			br#"{ "asset": { "default": { "asset": "materials/test.variant" } } }"#,
		);
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(TestVariantAssetHandler);
		asset_manager.add_asset_handler(FBXAssetHandler::new());

		let result = asset_manager.bake("degenerate_quad.fbx", &resource_storage).await;
		assert!(
			result.is_ok(),
			"valid geometry should remain after the degenerate quad is culled: {result:?}; trace: {:?}",
			asset_manager.resource_trace().items("degenerate_quad.fbx")
		);

		let items = asset_manager.resource_trace().items("degenerate_quad.fbx");
		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Info);
		assert_eq!(
			items[0].message(),
			"Culled degenerate FBX geometry: 0 triangle(s), 1 quad(s), and 0 other polygon(s). The most likely cause is repeated or collinear vertex positions, which produce zero-area triangles and undefined normal data."
		);
		assert!(resource_storage
			.get_resource(ResourceId::new("degenerate_quad.fbx"))
			.is_some());
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn malformed_fbx_keeps_its_handler_error_without_creating_a_resource() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("broken.fbx", b"not an FBX");
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(FBXAssetHandler::new());

		assert!(asset_manager.bake("broken.fbx", &resource_storage).await.is_err());

		assert!(resource_storage.get_resource(ResourceId::new("broken.fbx")).is_none());
		let items = asset_manager.resource_trace().items("broken.fbx");
		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Error);
		assert!(items[0].message().starts_with("Failed to import FBX asset 'broken.fbx':"));
	}

	#[r#async::test]
	async fn asset_manager_bakes_unfragmented_animation_only_fbx_as_animation() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("animation_only.fbx", ANIMATION_ONLY_FBX);
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(FBXAssetHandler::new());

		asset_manager
			.bake("animation_only.fbx", &resource_storage)
			.await
			.expect("an unfragmented animation-only FBX should bake as Animation");

		let animation = resource_storage
			.get_resource(crate::asset::ResourceId::new("animation_only.fbx"))
			.expect("the bare FBX Animation resource should be stored");

		assert_eq!(animation.class, "Animation");

		let animation = crate::from_slice::<AnimationModel>(&animation.resource).unwrap();

		assert_eq!(animation.skeleton.id().as_ref(), "animation_only.fbx#skeleton");
	}

	#[r#async::test]
	async fn bead_can_make_a_single_clip_fbx_with_geometry_default_to_animation() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("triangle_move.fbx", TRIANGLE_MOVE_FBX);
		asset_storage.add_file("triangle_move.fbx.bead", br#"{ "default_resource": "animation" }"#);
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(FBXAssetHandler::new());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx", &resource_storage)
			.await
			.expect("the BEAD default should override mesh-first FBX dispatch");

		assert_eq!(animation.class(), "Animation");
	}

	#[r#async::test]
	async fn asset_manager_bakes_explicit_fbx_skeleton_fragment_without_material_work() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("skinned_triangle.fbx", SKINNED_TRIANGLE_FBX);
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		asset_manager.add_asset_handler(FBXAssetHandler::new());

		let skeleton: ReferenceModel<SkeletonModel> = asset_manager
			.bake_if_not_exists("skinned_triangle.fbx#skeleton", &resource_storage)
			.await
			.expect("FBX skeleton fragment should bake without a shader generator");

		assert_eq!(skeleton.class(), "Skeleton");
		assert_eq!(skeleton.id().as_ref(), "skinned_triangle.fbx#skeleton");
	}

	#[r#async::test]
	async fn asset_manager_bakes_base_fbx_with_retained_skeleton_and_primitive_node() {
		let asset_storage = AssetTestStorageBackend::new();
		asset_storage.add_file("triangle_move.fbx", TRIANGLE_MOVE_FBX);
		let resource_storage = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage);
		let mut handler = FBXAssetHandler::new();
		handler.set_shader_generator(MinimalTestShaderGenerator);
		asset_manager.add_asset_handler(handler);

		let mesh: ReferenceModel<MeshModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx", &resource_storage)
			.await
			.expect("animated FBX base mesh should bake");
		let mesh = crate::from_slice::<MeshModel>(&mesh.resource).expect("animated FBX mesh should deserialize");

		assert_eq!(
			mesh.skeleton
				.as_ref()
				.expect("animated FBX mesh should retain its skeleton")
				.id()
				.as_ref(),
			"triangle_move.fbx#skeleton"
		);
		assert!(mesh.skins.is_empty());
		assert_eq!(mesh.primitives.len(), 1);
		assert!(mesh.primitives[0].transform_node.is_some());
		assert_eq!(mesh.primitives[0].skin, None);
	}

	/// Creates a serialized reference for fixture-local skeleton and animation imports.
	fn test_skeleton(model: &SkeletonModel) -> ReferenceModel<SkeletonModel> {
		ReferenceModel::new_serialized(
			"fixtures/model.fbx#skeleton",
			0,
			0,
			crate::to_vec(model).expect("fixture skeleton should serialize"),
			None,
		)
	}

	fn test_material(name: &str) -> ReferenceModel<VariantModel> {
		ReferenceModel::new_serialized(
			&format!("materials/{name}.variant"),
			0,
			0,
			crate::to_vec(&VariantModel {
				material: ReferenceModel::<MaterialModel>::new_serialized("materials/test.material", 0, 0, Vec::new(), None),
				variables: Vec::new(),
				alpha_mode: AlphaMode::Opaque,
			})
			.expect("test material should serialize"),
			None,
		)
	}

	/// Creates material references for every authored material in a parsed fixture scene.
	fn fixture_materials(scene: &ufbx::Scene) -> ResolvedFbxMaterials {
		ResolvedFbxMaterials {
			materials: scene
				.materials
				.iter()
				.map(|material| {
					(
						MaterialKey::Material(material.element.typed_id),
						test_material(material.element.name.as_ref()),
					)
				})
				.collect(),
		}
	}

	/// Computes the first triangle's signed XY area after applying MeshProcessor's clockwise index convention.
	fn first_clockwise_triangle_area(scene: &ufbx::Scene) -> f32 {
		let source =
			import_test_fbx_meshes(scene, &fixture_materials(scene), None, &[], &Global).expect("fixture mesh should import");
		let primitive = source.primitive(0).expect("fixture should contain a primitive");
		let Some(MeshAttributeData::F32x3(positions)) = primitive.attribute(VertexSemantics::Position, 0) else {
			panic!("FBX fixture should contain f32 position data");
		};
		let Some(MeshIndexData::U32(indices)) = primitive.indices(IndexStreamTypes::Triangles) else {
			panic!("FBX fixture should contain triangle indices");
		};
		let first = positions[indices[0] as usize];
		let second = positions[indices[2] as usize];
		let third = positions[indices[1] as usize];
		(second[0] - first[0]) * (third[1] - first[1]) - (second[1] - first[1]) * (third[0] - first[0])
	}

	/// Extracts the constant metallic-roughness values produced by the focused material fixtures.
	fn brdf_values(material: &BrdfMaterialDescription) -> ([f32; 4], f32, f32, [f32; 3]) {
		let BrdfNode::MetallicRoughness(surface) = material
			.node(material.surface)
			.expect("material surface should reference a node")
		else {
			panic!("FBX material should use a metallic-roughness surface");
		};
		let BrdfValue::Vector4(base_color) = constant_value(material, surface.base_color) else {
			panic!("FBX base color should be a vector4 constant");
		};
		let BrdfValue::Scalar(metallic) = constant_value(material, surface.metallic) else {
			panic!("FBX metalness should be a scalar constant");
		};
		let BrdfValue::Scalar(roughness) = constant_value(material, surface.roughness) else {
			panic!("FBX roughness should be a scalar constant");
		};
		let emission_node = surface.emission.expect("FBX material should contain an emission node");
		let BrdfNode::Emission { color } = material.node(emission_node).expect("emission should reference a node") else {
			panic!("FBX emission should use an emission node");
		};
		let BrdfValue::Vector3(emission) = constant_value(material, *color) else {
			panic!("FBX emission should be a vector3 constant");
		};
		(base_color, metallic, roughness, emission)
	}

	/// Reads one constant node from a fixture-generated BRDF graph.
	fn constant_value(material: &BrdfMaterialDescription, node: crate::pbr::BrdfNodeId) -> BrdfValue {
		match material.node(node).expect("constant should reference a node") {
			BrdfNode::Constant(value) => *value,
			_ => panic!("fixture BRDF value should be constant"),
		}
	}

	fn assert_vec3_close(actual: [f32; 3], expected: [f32; 3]) {
		for index in 0..3 {
			assert!((actual[index] - expected[index]).abs() < 1.0e-6);
		}
	}

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for index in 0..4 {
			assert!(
				(actual[index] - expected[index]).abs() < 1.0e-6,
				"component {index} differs: actual {actual:?}, expected {expected:?}"
			);
		}
	}

	fn assert_matrix_close(actual: [[f32; 4]; 4], expected: [[f32; 4]; 4]) {
		for column in 0..4 {
			assert_vec4_close(actual[column], expected[column]);
		}
	}
}
