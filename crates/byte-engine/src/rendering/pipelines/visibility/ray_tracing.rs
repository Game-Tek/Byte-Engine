//! The ray tracing pipeline that shades surfel hits, and the hit groups it runs for the scene's materials.
//!
//! Each material whose bake built a closest-hit shader runs a hit group: that shader with the constants that specialize
//! it. Groups keep the number they got when a material first asked for them, so a pipeline built again with more groups
//! runs every older group as before. Group 0 runs `surfel-default-hit.besl`, a black occluder, for every hit record
//! whose material's group isn't in the published pipeline yet. The pipeline compiles on the pipeline compiler threads,
//! and the published one keeps shading until the next one publishes.
//!
//! Next, [`super::render_pass::SurfelPool::prepare`] shades the frame's hits with the pipeline and the shader binding
//! table [`SurfelShading::prepare`] returns.

use ghi::context::ContextCreate as _;
use utils::hash::HashMap;

use crate::rendering::pipeline_compilation::{HitGroupShader, RayTracingPipelineRequest};
use crate::rendering::{PipelineManagerClient, PipelineRef, PipelineState};

/// The ray generation shader that shades surfel hits.
const SURFEL_SHADE_SHADER: &str = "byte-engine/rendering/visibility/surfel-shade.besl";
/// The closest-hit shader of hit group 0.
const DEFAULT_HIT_SHADER: &str = "byte-engine/rendering/visibility/surfel-default-hit.besl";
/// The stems of [`SURFEL_SHADE_SHADER`] and [`DEFAULT_HIT_SHADER`], which a bake leaves out with the rest of surfel GI.
/// They have no pipeline asset, because [`SurfelShading`] assembles the pipeline from them.
pub(super) const SURFEL_SHADING_SHADERS: [&str; 2] = ["surfel-shade", "surfel-default-hit"];

/// The `HitGroups` struct numbers the hit groups of the surfel shading pipeline, so each closest-hit shader with its
/// constants keeps one group for as long as the renderer runs.
///
/// Materials that share a shader and its constants share its group. A pipeline request lists [`Self::shaders`], whose
/// positions are the group numbers.
pub(crate) struct HitGroups {
	shaders: Vec<HitGroupShader>,
	groups: HashMap<HitGroupShader, u32>,
}

impl HitGroups {
	/// Creates the table with `default` as group 0.
	pub(crate) fn new(default: HitGroupShader) -> Self {
		let mut groups = HashMap::default();
		groups.insert(default.clone(), 0);
		Self {
			shaders: vec![default],
			groups,
		}
	}

	/// Returns the group of `shader`, adding it after every other group when it is new.
	pub(crate) fn group(&mut self, shader: &HitGroupShader) -> u32 {
		if let Some(&group) = self.groups.get(shader) {
			return group;
		}
		let group = self.shaders.len() as u32;
		self.shaders.push(shader.clone());
		self.groups.insert(shader.clone(), group);
		group
	}

	/// Returns each group's closest-hit shader, in group order.
	pub(crate) fn shaders(&self) -> &[HitGroupShader] {
		&self.shaders
	}
}

/// Writes into `groups` the hit group each hit record runs: the group of its material when the published pipeline has
/// that group, and group 0 otherwise.
///
/// `record_materials` lists each record's material slot, and `material_groups` each slot's group, 0 for a material
/// without a closest-hit shader. `published` is how many groups the published pipeline has.
pub(crate) fn record_hit_groups(record_materials: &[u32], material_groups: &[u32], published: u32, groups: &mut Vec<u32>) {
	groups.clear();
	groups.extend(record_materials.iter().map(|&material| {
		let group = material_groups.get(material as usize).copied().unwrap_or(0);
		if group < published { group } else { 0 }
	}));
}

/// The `SurfelShading` struct keeps the ray tracing pipeline that shades surfel hits current with the materials the
/// scene loads, and the shader binding table that maps each hit record to its material's hit group.
///
/// The visibility pipeline manager creates it with the surfel pool. Give it each material's hit group with
/// [`Self::set_material`] as the material loads, and each frame surfel GI may light, call [`Self::prepare`] for the
/// pipeline and table to shade with.
pub(crate) struct SurfelShading {
	hit_groups: HitGroups,
	/// Whether each group's shader failed to compile, so requests run the default shader in its place.
	broken: Vec<bool>,
	/// Each material slot's group, 0 for a material without a closest-hit shader.
	material_groups: Vec<u32>,
	/// Counts changes to `material_groups`, so the table is written again after one.
	material_revision: u64,
	/// Whether the groups changed since the last request.
	request_outdated: bool,
	/// The last request, how many groups it has, and when it was made, until it publishes or fails.
	pending: Option<(PipelineRef, u32, std::time::Instant)>,
	/// The request that published last and how many groups it has.
	published: Option<(PipelineRef, u32)>,
	table: ghi::ShaderBindingTableHandle,
	/// What the table holds: the pipeline it was written for, how many records, and the material revision.
	written: Option<(ghi::PipelineHandle, usize, u64)>,
	/// Each record's group, kept so writes reuse the allocation.
	record_groups: Vec<u32>,
}

impl SurfelShading {
	/// Creates the shader binding table and requests a first pipeline, with the default hit group only.
	pub(crate) fn new(context: &mut ghi::implementation::Context) -> Self {
		Self {
			hit_groups: HitGroups::new(HitGroupShader {
				shader_id: DEFAULT_HIT_SHADER.to_owned(),
				specialization: Vec::new(),
			}),
			broken: vec![false],
			material_groups: Vec::new(),
			material_revision: 0,
			request_outdated: true,
			pending: None,
			published: None,
			table: context.create_shader_binding_table(Some("Surfel Hit Records")),
			written: None,
			record_groups: Vec::new(),
		}
	}

	/// Sets the hit group that hits on the material in `slot` run: `shader`'s, or the default one for `None`.
	pub(crate) fn set_material(&mut self, slot: u32, shader: Option<&HitGroupShader>) {
		let groups = self.hit_groups.shaders().len();
		let group = shader.map_or(0, |shader| self.hit_groups.group(shader));
		if self.hit_groups.shaders().len() != groups {
			self.broken.push(false);
			self.request_outdated = true;
		}
		let slot = slot as usize;
		if slot >= self.material_groups.len() {
			self.material_groups.resize(slot + 1, 0);
		}
		if self.material_groups[slot] != group {
			self.material_groups[slot] = group;
			self.material_revision += 1;
		}
	}

	/// Returns the pipeline and the shader binding table to shade this frame's hits with, or `None` until a pipeline
	/// publishes.
	///
	/// `record_materials` lists the material slot of each hit record, as
	/// [`super::acceleration_structures::SceneAccelerationStructures::admit`] returns them this frame. Once the last
	/// request settled, a new one asks for every group added since. The table is written again when the pipeline, the
	/// records, or a material's group changed.
	pub(crate) fn prepare(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		record_materials: &[u32],
	) -> Option<(ghi::PipelineHandle, ghi::ShaderBindingTableHandle)> {
		self.settle_request(pipeline_manager);
		if self.pending.is_none() && self.request_outdated {
			self.request(pipeline_manager);
		}

		let (published, groups) = self.published?;
		// A development rebake publishes the same request again under a new handle.
		let pipeline = pipeline_manager.pipeline(published)?;
		let contents = (pipeline, record_materials.len(), self.material_revision);
		if self.written != Some(contents) {
			record_hit_groups(record_materials, &self.material_groups, groups, &mut self.record_groups);
			frame.write_shader_binding_table(self.table, pipeline, &self.record_groups);
			self.written = Some(contents);
		}
		Some((pipeline, self.table))
	}

	/// Publishes the pending request once it compiled. When it failed, the groups it added run the default shader from
	/// then on.
	fn settle_request(&mut self, pipeline_manager: &PipelineManagerClient) {
		let Some((pipeline, groups, requested)) = self.pending else {
			return;
		};
		match pipeline_manager.get(pipeline) {
			PipelineState::Pending => {}
			PipelineState::Ready(_) => {
				log::debug!(
					"The surfel GI shading pipeline published with {groups} hit groups, {:.1} ms after its request.",
					requested.elapsed().as_secs_f64() * 1000.0
				);
				self.published = Some((pipeline, groups));
				self.pending = None;
			}
			PipelineState::Failed => {
				let published = self.published.map_or(0, |(_, groups)| groups);
				log::warn!(
					"The surfel GI shading pipeline with {groups} hit groups failed to compile. The most likely cause is that the closest-hit shader of a material that loaded since the last pipeline doesn't compile. Hits on the {} materials' hit groups added since then shade as black occluders; check the shader compilation errors logged before this warning, and rebake the materials.",
					groups - published
				);
				for broken in &mut self.broken[published as usize..groups as usize] {
					*broken = true;
				}
				self.pending = None;
				// A request without the failed shaders can still publish the groups that compile.
				self.request_outdated = true;
			}
		}
	}

	/// Requests a pipeline with every group, the failed ones running the default shader.
	fn request(&mut self, pipeline_manager: &PipelineManagerClient) {
		let default = &self.hit_groups.shaders()[0];
		let shaders = self
			.hit_groups
			.shaders()
			.iter()
			.zip(&self.broken)
			.map(|(shader, &broken)| if broken { default.clone() } else { shader.clone() })
			.collect::<Vec<_>>();
		let groups = shaders.len() as u32;
		let request = RayTracingPipelineRequest::new(SURFEL_SHADE_SHADER, shaders);
		self.pending = Some((
			pipeline_manager.request_ray_tracing_pipeline(request),
			groups,
			std::time::Instant::now(),
		));
		self.request_outdated = false;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hit_shader(id: &str, constant: u32) -> HitGroupShader {
		HitGroupShader {
			shader_id: id.to_owned(),
			specialization: vec![ghi::pipelines::SpecializationMapEntry::new(0, constant)],
		}
	}

	#[test]
	fn hit_groups_keep_their_numbers_as_shaders_arrive() {
		let default = hit_shader("default", 0);
		let mut groups = HitGroups::new(default.clone());
		let stone = hit_shader("stone", 0);
		let painted_stone = hit_shader("stone", 1);

		assert_eq!(groups.group(&stone), 1);
		assert_eq!(groups.group(&painted_stone), 2);
		// A second material with the same shader and constants shares the group.
		assert_eq!(groups.group(&stone), 1);
		assert_eq!(groups.group(&default), 0);
		assert!(groups.shaders() == [default, stone, painted_stone]);
	}

	#[test]
	fn records_run_their_material_group_only_once_the_pipeline_has_it() {
		// Material slot 0 has no closest-hit shader, slot 1 runs group 1, slot 2 runs group 2, and slot 3 is unknown.
		let material_groups = [0, 1, 2];
		let record_materials = [1, 2, 0, 3, 2];
		let mut groups = Vec::new();

		record_hit_groups(&record_materials, &material_groups, 3, &mut groups);
		assert_eq!(groups, [1, 2, 0, 0, 2]);
		// A pipeline built before group 2 existed runs the default group for its records.
		record_hit_groups(&record_materials, &material_groups, 2, &mut groups);
		assert_eq!(groups, [1, 0, 0, 0, 0]);
	}
}
