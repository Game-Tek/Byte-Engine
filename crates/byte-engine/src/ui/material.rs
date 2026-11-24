//! The UI material domain: how a BEMX graph becomes the fragment stage of a UI material fill.
//!
//! A `.bema` material whose `domain` is [`UI_DOMAIN`] bakes through [`UiShaderGenerator`] into a fragment shader that
//! the UI render pass draws with its own pipeline, apart from the ubershader that draws plain fills. An element paints a
//! layer with it through [`Material`](crate::ui::style::Material). Next, see [`crate::ui::style::Material::new`].

#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]

use besl::parser::{Node, Nodes};
use resource_management::asset::handler::implementations::bema::{
	MaterialParameter, ParameterKind, ProgramGenerator, ProgramModules,
};
use resource_management::bemx::{
	self,
	besl::{Coordinates, Input, LowerError, Lowering, Stage, component, construct},
};
use resource_management::shader::generator::ShaderGenerationSettings;

/// The material domain the UI draws. A `.bema` material with `"domain": "UI"` bakes through [`UiShaderGenerator`].
pub const UI_DOMAIN: &str = "UI";

/// The pipeline every UI material's fragment shader completes: the UI vertex stage, with the material's shader as its
/// fragment stage.
pub(crate) const UI_MATERIAL_PIPELINE: &str = "byte_engine/rendering/ui/ui-material.pipeline";

/// The modules a UI material's program brings in: the UI layer, and the shared coverage of a material fill.
const UI_MATERIAL_MODULES: [&str; 2] = [
	"::byte_engine::rendering::ui::layouts::ui",
	"::byte_engine::rendering::ui::material",
];

/// The module sets the generator's programs bring in; see [`ProgramGenerator::module_sets`].
const MODULE_SETS: [&[&str]; 1] = [&UI_MATERIAL_MODULES];

/// The fragment entry point every UI material shares. The generator splices the graph's statements in at
/// [`GRAPH_POSITION`], where they overwrite `base_color` and `opacity`.
///
/// The locals before it are what [`UiStage`] lowers the graph's inputs against. The record's `gradient` slot carries the
/// element's pixels per unit and where its parameter entries start, since a material fill has no gradient.
const UI_MATERIAL_SOURCE: &str = r#"
ui_material: fn (
	pipeline_input: interface {
		curve_from: vec4f,
		curve_to: vec2f,
		pixel_position: vec2f,
		primitive: u32,
		uv: vec2f,
		backdrop_position: vec2f,
	},
) -> output { color_attachment: vec4f } {
	let primitive: u32 = pipeline_input.primitive;
	let pixel: vec2f = pipeline_input.pixel_position;
	let coverage: f32 = material_coverage(ui, primitive, pixel);
	let element: vec4f = ui.primitives[primitive].a;
	let placement: vec4f = ui.primitives[primitive].gradient;
	let pixel_scale: vec2f = vec2f(placement.x, placement.y);
	let element_uv: vec2f = vec2f((pixel.x - element.x) / max(element.z, 0.0001), (pixel.y - element.y) / max(element.w, 0.0001));
	let element_position: vec3f = vec3f((pixel.x - element.x) / pixel_scale.x, (pixel.y - element.y) / pixel_scale.y, 0.0);
	let element_size: vec2f = vec2f(element.z / pixel_scale.x, element.w / pixel_scale.y);
	let parameters: u32 = u32(placement.z);
	let base_color: vec3f = vec3f(0.8, 0.8, 0.8);
	let opacity: f32 = 1.0;
	// The record's color alpha carries the element's opacity.
	let color_attachment: vec4f = vec4f(base_color.x, base_color.y, base_color.z, clamp(opacity * coverage * f32(ui.primitives[primitive].color.w), 0.0, 1.0));
	return { color_attachment };
}
"#;

/// How many statements of [`UI_MATERIAL_SOURCE`] come before the color it writes, which is where the graph goes.
const GRAPH_POSITION: usize = 12;

/// The surface inputs a UI material reads. A graph that connects any other fails to bake, so nothing it computes is
/// silently dropped.
const UI_SURFACE_INPUTS: [&str; 2] = ["base_color", "opacity"];

/// The `UiShaderGenerator` struct turns the BEMX graphs of [`UI_DOMAIN`] materials into UI material fragment shaders.
///
/// Install it for the `UI` domain with
/// [`AssetManager::set_program_generator`](resource_management::asset::AssetManager::set_program_generator), as
/// [`configure_default_baking`](crate::application::graphics::defaults::configure_default_baking) does. The UI render
/// pass then requests each material's shader as the fragment stage of the UI material pipeline.
#[derive(Clone, Copy, Debug, Default)]
pub struct UiShaderGenerator;

impl UiShaderGenerator {
	pub fn new() -> Self {
		Self
	}
}

impl ProgramGenerator for UiShaderGenerator {
	fn identity(&self) -> u64 {
		use std::hash::{Hash as _, Hasher as _};

		// The entry point is the only thing besides the graph and the modules that shapes a material's program.
		let mut hasher = utils::hash::FxHasher::with_seed(0x55_49_4d_41_54);
		UI_MATERIAL_SOURCE.hash(&mut hasher);
		UI_SURFACE_INPUTS.hash(&mut hasher);
		hasher.finish()
	}

	fn module_sets(&self) -> &[&[&str]] {
		&MODULE_SETS
	}

	fn generate<'a>(
		&self,
		graph: &'a bemx::Graph,
		parameters: &'a [MaterialParameter],
		modules: &'a ProgramModules,
	) -> Result<Option<(Node<'a>, ShaderGenerationSettings)>, String> {
		let program = ui_material_program(graph, parameters, modules.get(&UI_MATERIAL_MODULES))?;
		Ok(Some((program, ShaderGenerationSettings::fragment())))
	}
}

/// Builds the fragment program of a UI material from its graph and parameters, bringing in the loaded `files` of
/// [`UI_MATERIAL_MODULES`].
fn ui_material_program<'a>(
	graph: &'a bemx::Graph,
	parameters: &'a [MaterialParameter],
	files: &'a [besl::LoadedModule],
) -> Result<Node<'a>, String> {
	let surface = graph.surface();
	if let Some((name, ..)) = surface
		.inputs()
		.into_iter()
		.find(|(name, input, ..)| input.is_some() && !UI_SURFACE_INPUTS.contains(name))
	{
		return Err(format!(
			"A UI material's graph sets the surface's `{name}`, which UI materials don't read. The most likely cause is a graph written for the `World` domain. Compute the color into `base_color` and the coverage into `opacity`, and leave `{name}` unconnected. See {}.",
			crate::online_docs_url(UI_MATERIAL_DOCS_PATH)
		));
	}
	if let Some(texture) = parameters.iter().find(|parameter| parameter.kind == ParameterKind::Texture) {
		return Err(format!(
			"UI material parameter '{}' is a texture, which UI materials can't sample yet. The most likely cause is a material written for the `World` domain. Remove the `Texture2D` variable and the `image` nodes that read it. See {}.",
			texture.name,
			crate::online_docs_url(UI_MATERIAL_DOCS_PATH)
		));
	}

	// Only what `base_color` and `opacity` reach is lowered.
	let mut lowering = Lowering::new(graph, UiStage { parameters });
	let mut outputs = Vec::with_capacity(2);
	for (name, input) in [("base_color", surface.base_color), ("opacity", surface.opacity)] {
		if let Some(input) = input {
			outputs.push((name, lowering.value(input).map_err(ui_lower_error)?.syntax));
		}
	}
	let (mut statements, _) = lowering.finish();
	statements.extend(outputs.into_iter().map(|(name, value)| Node::member_assignment(name, value)));

	let mut main = besl::parse(UI_MATERIAL_SOURCE)
		.ok()
		.and_then(|mut root| match root.node_mut() {
			Nodes::Scope { children, .. } if children.len() == 1 => Some(children.remove(0)),
			_ => None,
		})
		.expect("The UI material entry point is valid BESL with one function.");
	let Nodes::Function {
		name,
		params,
		statements: body,
		..
	} = main.node_mut()
	else {
		unreachable!("The UI material entry point is a function.");
	};
	*name = "main";
	params.push(Node::parameter("ui", "Ui"));
	body.splice(GRAPH_POSITION..GRAPH_POSITION, statements);

	let mut children: Vec<Node<'a>> = UI_MATERIAL_MODULES.iter().map(|module| Node::r#use(module, true)).collect();
	children.push(main);
	let mut root = Node::root_with_children(children);
	besl::attach_modules(&mut root, files).map_err(|error| {
		format!(
			"Failed to attach the UI modules to a UI material's shader ({error:?}). The most likely cause is a material parameter named like a declaration of the UI modules. Rename the parameter."
		)
	})?;
	Ok(root)
}

/// Rewords a lowering error for the UI domain, whose fragments carry other inputs than the scene's.
fn ui_lower_error(error: LowerError) -> String {
	let docs = crate::online_docs_url(UI_MATERIAL_DOCS_PATH);
	match error {
		LowerError::UnavailableInput { node, input } => format!(
			"BEMX node #{} reads the {input:?} input, which UI materials don't carry. The most likely cause is a graph written for the `World` domain. Read `texcoord`, `position`, or the `size` and `pixelscale` geometric properties instead. See {docs}.",
			node.index()
		),
		LowerError::UnavailableBackdrop { node } => format!(
			"BEMX node #{} reads the scene behind the surface, which UI materials can't read. The most likely cause is a graph written for the `World` domain. Remove the `scenecolor` or `sceneposition` node. See {docs}.",
			node.index()
		),
		error => error.to_string(),
	}
}

/// The reference page that specifies what a UI material reads.
const UI_MATERIAL_DOCS_PATH: &str = "reference/bemx#ui-domain";

/// The `UiStage` struct supplies what a BEMX graph reads in a UI material fill: where the pixel lies in its element,
/// the element's size and scale, and the material's parameters.
///
/// The values are the locals [`UI_MATERIAL_SOURCE`] declares.
struct UiStage<'a> {
	parameters: &'a [MaterialParameter],
}

impl<'a> Stage<'a> for UiStage<'a> {
	fn input(&mut self, input: Input) -> Option<Node<'a>> {
		match input {
			Input::TexCoord => Some(Node::member_expression("element_uv")),
			Input::Position => Some(Node::member_expression("element_position")),
			Input::Normal | Input::Tangent | Input::Bitangent | Input::View | Input::Time | Input::ScreenPosition => None,
		}
	}

	fn property(&mut self, name: &'a str, r#type: bemx::Type) -> Option<Node<'a>> {
		match (name, r#type) {
			("size", bemx::Type::Vector2) => Some(Node::member_expression("element_size")),
			("pixelscale", bemx::Type::Vector2) => Some(Node::member_expression("pixel_scale")),
			_ => None,
		}
	}

	fn sample(&mut self, texture: &'a str, _: Coordinates<'a>) -> Result<Node<'a>, LowerError> {
		// `ui_material_program` rejects every texture parameter before lowering, so any image names an unknown one.
		Err(LowerError::UnknownTexture {
			name: texture.to_string(),
		})
	}

	fn parameter(&mut self, name: &'a str, r#type: bemx::Type) -> Result<Node<'a>, LowerError> {
		// Every value parameter, static or dynamic, takes one entry of the element's block, in declaration order: an
		// element can override any of them, and the block starts at `parameters`.
		let entry = self
			.parameters
			.iter()
			.filter(|parameter| matches!(parameter.kind, ParameterKind::Value(_) | ParameterKind::Dynamic(_)))
			.position(|parameter| {
				parameter.name == name
					&& matches!(parameter.kind, ParameterKind::Value(declared) | ParameterKind::Dynamic(declared) if declared == r#type)
			})
			.ok_or_else(|| LowerError::UnknownParameter {
				name: name.to_string(),
				r#type,
			})?;
		let index = Node::operator(
			"+",
			Node::member_expression("parameters"),
			Node::literal_expression(format!("{entry}u")),
		);
		let value = Node::accessor(
			Node::accessor(Node::member_expression("ui"), Node::member_expression("material_parameters")),
			Node::sentence(vec![index]),
		);
		Ok(construct(
			(0..r#type.width()).map(|index| component(&value, index, 4)).collect(),
		))
	}
}

/// The `UiMaterialLayout` struct is what the UI render pass needs of a baked UI material to draw it: the fragment
/// shader its pipeline completes, and the parameter entries every element that paints with it fills, in the order the
/// shader reads them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UiMaterialLayout {
	/// The ID of the material's fragment shader.
	pub(crate) shader: String,
	/// Each value parameter's name and the material's own value, which an element that sets none keeps.
	pub(crate) entries: Vec<(String, crate::ui::style::MaterialValue)>,
}

/// Reads what the UI render pass draws a stored UI material with, or the error to report when it can't.
fn material_layout(
	id: &str,
	material: &resource_management::resources::material::Material,
) -> Result<UiMaterialLayout, String> {
	use resource_management::resources::material::Value;

	let docs = crate::online_docs_url(UI_MATERIAL_DOCS_PATH);
	if material.domain != UI_DOMAIN {
		return Err(format!(
			"Material '{id}' belongs to the '{}' domain, so the UI can't paint with it. The most likely cause is a scene material named in a UI layer. Paint with a material whose `domain` is `UI`. See {docs}.",
			material.domain
		));
	}
	let shader = material
		.shaders()
		.iter()
		.find(|shader| shader.resource().stage == resource_management::types::ShaderTypes::Fragment)
		.ok_or_else(|| {
			format!(
				"UI material '{id}' has no fragment shader. The most likely cause is a material baked before the UI domain existed. Bake it again with `beld bake --force`. See {docs}."
			)
		})?;
	let entries = material
		.parameters
		.iter()
		.filter_map(|parameter| {
			let value = match parameter.value {
				Value::Scalar(value) => value.into(),
				Value::Vector2(value) => value.into(),
				Value::Vector3(value) => value.into(),
				Value::Vector4(value) => value.into(),
				Value::Image(_) => return None,
			};
			Some((parameter.name.clone(), value))
		})
		.collect();
	Ok(UiMaterialLayout {
		shader: shader.id().to_string(),
		entries,
	})
}

/// The `UiMaterialLoader` struct reads the UI materials elements paint with on the loading thread, so the render
/// thread never waits for a `.bema` to bake or load.
///
/// [`setup_ui_render_pass`](crate::application::graphics::setup_ui_render_pass) spawns it once per application, on the
/// same loader lanes as the scene's materials; every UI render pass reads it through [`UiMaterials`].
pub(crate) struct UiMaterialLoader {
	pub(crate) resources: crate::core::EntityHandle<resource_management::ResourceManager>,
}

impl crate::rendering::loading::LoadPipeline for UiMaterialLoader {
	type Key = String;
	type Request = String;
	type Resident = UiMaterialLayout;

	fn key(request: &Self::Request) -> Self::Key {
		request.clone()
	}

	/// Reads one material's record. Its pipeline compiles separately, through the renderer's pipeline compiler.
	async fn load(
		&self,
		id: String,
		_lane: &mut crate::rendering::loading::LoaderLane<Self>,
	) -> Result<UiMaterialLayout, crate::rendering::loading::LoadError> {
		let material = self
			.resources
			.request::<resource_management::resources::material::Material>(&id)
			.await
			.map_err(|error| {
				crate::rendering::loading::LoadError(format!(
					"UI material '{id}' failed to load: {error}. The most likely cause is a misspelt material ID or a material that failed to bake. Check the ID against the asset's path, and the bake log. See {}.",
					crate::online_docs_url(UI_MATERIAL_DOCS_PATH)
				))
			})?;
		material_layout(&id, material.resource()).map_err(crate::rendering::loading::LoadError)
	}
}

/// The `UiMaterialRead` enum is how far reading one UI material got.
pub(crate) enum UiMaterialRead {
	/// The material is still being read.
	Reading,
	Ready(UiMaterialLayout),
	/// The material can't be drawn; the reason was logged when the read failed.
	Failed,
}

/// The `UiMaterials` struct is the render thread's view of the application's UI material loader, which every UI
/// render pass shares, so a material is read once however many windows paint with it.
///
/// [`GraphicsApplication`](crate::application::graphics::GraphicsApplication) makes it with the first UI render pass;
/// clones share the same reads. [`Self::none`] reads nothing, for a pass that paints no materials.
#[derive(Clone)]
pub struct UiMaterials(Option<std::rc::Rc<std::cell::RefCell<UiMaterialCatalog>>>);

/// The `UiMaterialCatalog` struct holds the loader's client and every read it finished.
struct UiMaterialCatalog {
	client: crate::rendering::loading::LoaderClient<UiMaterialLoader>,
	/// Each finished read: the layout, or `None` for a read that failed.
	reads: utils::hash::HashMap<String, Option<UiMaterialLayout>>,
}

impl UiMaterials {
	/// Reads materials through `client`, the render-thread half of a spawned [`UiMaterialLoader`].
	pub(crate) fn new(client: crate::rendering::loading::LoaderClient<UiMaterialLoader>) -> Self {
		Self(Some(std::rc::Rc::new(std::cell::RefCell::new(UiMaterialCatalog {
			client,
			reads: Default::default(),
		}))))
	}

	/// Reads no materials: every material an element paints with fails, and the pass leaves it out.
	pub fn none() -> Self {
		Self(None)
	}

	/// Starts reading the material `id`, unless it is read or being read. Next, follow it with [`Self::read`].
	pub(crate) fn request(&self, id: &str) {
		let Some(catalog) = &self.0 else {
			log::error!(
				"UI material '{id}' can't be read. The most likely cause is a UI render pass built without a material loader, such as one set up after the loading thread started while no earlier UI pass had made the loader. Set up a UI render pass before `launch_deferred_tasks_thread`."
			);
			return;
		};
		let mut catalog = catalog.borrow_mut();
		if !catalog.reads.contains_key(id) {
			catalog.client.request(id.to_string());
		}
	}

	/// Returns how far reading the material `id` got, taking every read the loader finished first.
	pub(crate) fn read(&self, id: &str) -> UiMaterialRead {
		let Some(catalog) = &self.0 else {
			return UiMaterialRead::Failed;
		};
		let mut catalog = catalog.borrow_mut();
		while let Some(event) = catalog.client.poll() {
			match event {
				crate::rendering::loading::Event::Ready { key, resident } => {
					catalog.reads.insert(key, Some(resident));
				}
				crate::rendering::loading::Event::Failed { key, error } => {
					log::error!("{error}");
					catalog.reads.insert(key, None);
				}
			}
		}
		match catalog.reads.get(id) {
			None => UiMaterialRead::Reading,
			Some(Some(layout)) => UiMaterialRead::Ready(layout.clone()),
			Some(None) => UiMaterialRead::Failed,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Returns the modules a UI material's program brings in, embedded from the engine's assets.
	fn modules() -> ProgramModules {
		const FILES: [(&str, &str); 3] = [
			(
				"byte_engine/rendering/ui/layouts/ui.besl",
				include_str!("../../assets/rendering/ui/layouts/ui.besl"),
			),
			(
				"byte_engine/rendering/ui/material.besl",
				include_str!("../../assets/rendering/ui/material.besl"),
			),
			(
				"byte_engine/rendering/ui/shading.besl",
				include_str!("../../assets/rendering/ui/shading.besl"),
			),
		];
		ProgramModules::load(&MODULE_SETS, |file| {
			FILES
				.iter()
				.find(|(path, _)| *path == file)
				.map(|(_, source)| (*source).to_string())
		})
		.expect("The UI material modules failed to load.")
	}

	fn grid_parameters() -> Vec<MaterialParameter> {
		[
			("pan", ParameterKind::Dynamic(bemx::Type::Vector2)),
			("zoom", ParameterKind::Dynamic(bemx::Type::Float)),
			("spacing", ParameterKind::Value(bemx::Type::Float)),
			("minor_color", ParameterKind::Value(bemx::Type::Vector4)),
			("major_color", ParameterKind::Value(bemx::Type::Vector4)),
		]
		.into_iter()
		.map(|(name, kind)| MaterialParameter {
			name: name.to_string(),
			kind,
		})
		.collect()
	}

	/// Generates the program of `graph`, then links it and lowers it to the platform's shading language as a fragment
	/// shader, which is how a bake compiles it.
	fn lower(graph: &str, parameters: &[MaterialParameter]) -> Result<(), String> {
		let builtins = besl::Builtins::new();
		let modules = modules();
		let graph = bemx::json::parse(graph).map_err(|error| error.to_string())?;
		let (program, settings) = UiShaderGenerator
			.generate(&graph, parameters, &modules)?
			.expect("UI materials always build a shader.");
		let mut lexed = besl::lex(&builtins, program).map_err(|error| format!("{error:?}"))?;
		resource_management::shader::besl::backends::platform::PlatformShaderCompiler::new()
			.lower(&settings, &mut lexed)
			.map(|_| ())
			.map_err(|error| format!("{error:?}"))
	}

	#[test]
	fn graph_grid_material_lowers_to_a_fragment_shader() {
		lower(include_str!("../../assets/editor/graph-grid.bemx"), &grid_parameters()).unwrap();
	}

	#[test]
	fn reading_scene_geometry_fails_with_the_ui_domain_message() {
		let error = lower(
			r#"{ nodes: [
				{ name: "n", node: "normal" },
				{ name: "surface", node: "surface", base_color: "n" },
			] }"#,
			&[],
		)
		.unwrap_err();
		assert!(error.contains("which UI materials don't carry"), "{error}");
	}

	#[test]
	fn connecting_an_input_ui_materials_do_not_read_fails() {
		let error = lower(
			r#"{ nodes: [
				{ name: "surface", node: "surface", emission: [1, 0, 0] },
			] }"#,
			&[],
		)
		.unwrap_err();
		assert!(error.contains("`emission`"), "{error}");
	}
}
