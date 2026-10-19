//! The values of dynamic material parameters, which material evaluation reads from the `material_parameters` buffer.
//!
//! Each loaded material with dynamic parameters owns a block of entries, one per parameter in declaration order. An
//! entity the application set a parameter on gets its own block per material it renders with: the material's values
//! with the entity's written over them. [`MaterialParameters`] keeps the values; the visibility manager copies the
//! changed entries into each frame's buffer and names each instance's block in its record.

use std::borrow::Cow;
use std::hash::{Hash, Hasher as _};
use std::ops::Range;
use std::sync::Arc;

use log::warn;
use resource_management::bemx;
use smallvec::SmallVec;
use utils::hash::{FxHasher, HashMap, HashSet};

use super::layout::MAX_MATERIAL_PARAMETERS;
use super::loader::DynamicParameter;
use super::ranges::RangeAllocator;
use crate::core::factory::Handle;
use crate::rendering::{MaterialParameterTarget, SetMaterialParameter};

/// The `MaterialParameters` struct lets the visibility manager give every instance the parameter values it should
/// shade with, and upload only what changed.
pub(crate) struct MaterialParameters {
	entries: Entries,
	ranges: RangeAllocator,
	/// The block of each loaded material, by material table slot.
	materials: Vec<Option<MaterialBlock>>,
	/// The values set on each material, by resource ID, and the slot its loaded copy takes.
	by_id: HashMap<Arc<str>, MaterialValues>,
	/// The values set on each entity, and the blocks its primitives read.
	entities: HashMap<Handle, EntityValues>,
	/// Hashes of the warnings already reported, so a value set every frame warns once.
	reported: HashSet<u64>,
}

/// The values applications set on one target, latest per parameter name.
type Values = Vec<(Cow<'static, str>, bemx::Value)>;

/// The `MaterialBlock` struct places one loaded material's parameters in the buffer.
struct MaterialBlock {
	id: Arc<str>,
	parameters: Arc<[DynamicParameter]>,
	range: Range<u32>,
}

/// The `MaterialValues` struct keeps what applications set on one material, loaded or not.
#[derive(Default)]
struct MaterialValues {
	slot: Option<u32>,
	values: Values,
}

/// The `EntityValues` struct keeps what applications set on one entity and the blocks that hold it.
#[derive(Default)]
struct EntityValues {
	values: Values,
	blocks: SmallVec<[EntityBlock; 2]>,
}

/// The `EntityBlock` struct places one entity's values for one of its materials in the buffer.
struct EntityBlock {
	material: u32,
	range: Range<u32>,
	/// Whether the entity's values or its material's changed since the block was written.
	stale: bool,
}

impl MaterialParameters {
	pub(crate) fn new() -> Self {
		Self {
			entries: Entries {
				values: vec![[0.0; 4]; MAX_MATERIAL_PARAMETERS].into_boxed_slice(),
				dirty: std::array::from_fn(|_| 0..0),
			},
			ranges: RangeAllocator::new(MAX_MATERIAL_PARAMETERS as u32),
			materials: Vec::new(),
			by_id: HashMap::default(),
			entities: HashMap::default(),
			reported: HashSet::default(),
		}
	}

	/// Gives the material `id`, loaded at table slot `slot`, a block holding its defaults and the values set on it so
	/// far, replacing the block of the copy it reloads.
	pub(crate) fn adopt_material(&mut self, slot: u32, id: Arc<str>, parameters: Arc<[DynamicParameter]>) {
		let index = slot as usize;
		if index >= self.materials.len() {
			self.materials.resize_with(index + 1, || None);
		}
		if let Some(previous) = self.materials[index].take() {
			self.ranges.free(previous.range);
		}
		let range = self.ranges.allocate(parameters.len() as u32).unwrap_or_else(|| {
			report(&mut self.reported, Warning::NoRoom);
			0..0
		});
		for (entry, parameter) in range.clone().zip(parameters.iter()) {
			self.entries.write(entry, parameter.default);
		}
		let material_values = self.by_id.entry(id.clone()).or_default();
		material_values.slot = Some(slot);
		for (name, value) in &material_values.values {
			if let Err(error) = write(&mut self.entries, &parameters, range.clone(), name, *value) {
				report(&mut self.reported, error.warning(&id, name, *value));
			}
		}
		self.materials[index] = Some(MaterialBlock { id, parameters, range });
		self.mark_material_users_stale(slot);
	}

	/// Applies a value the application set. A value set on a material that isn't loaded yet applies once it loads.
	pub(crate) fn receive(&mut self, message: SetMaterialParameter) {
		let SetMaterialParameter { target, name, value } = message;
		match target {
			MaterialParameterTarget::Entity(handle) => {
				let entity = self.entities.entry(handle).or_default();
				if set(&mut entity.values, &name, value) {
					entity.blocks.iter_mut().for_each(|block| block.stale = true);
				}
			}
			MaterialParameterTarget::Material(id) => {
				let material_values = match self.by_id.get_mut(&*id) {
					Some(material_values) => material_values,
					None => self.by_id.entry(Arc::from(&*id)).or_default(),
				};
				if !set(&mut material_values.values, &name, value) {
					return;
				}
				let Some(slot) = material_values.slot else {
					return;
				};
				let Some(block) = self.materials[slot as usize].as_ref() else {
					return;
				};
				match write(&mut self.entries, &block.parameters, block.range.clone(), &name, value) {
					Ok(()) => self.mark_material_users_stale(slot),
					Err(error) => report(&mut self.reported, error.warning(&block.id, &name, value)),
				}
			}
		}
	}

	/// Returns the first entry of the block one primitive of `handle`, drawn with the material at table slot
	/// `material`, reads: its own when the application set values on `handle`, else its material's.
	///
	/// An entity's block is written here, on the first frame that needs it and after its values or its material's
	/// change.
	pub(crate) fn block(&mut self, handle: Handle, material: u32) -> u32 {
		let Some(material_block) = self.materials.get(material as usize).and_then(Option::as_ref) else {
			return 0;
		};
		let shared = material_block.range.start;
		let length = material_block.parameters.len();
		let Some(entity) = self.entities.get_mut(&handle) else {
			return shared;
		};
		if length == 0 {
			return shared;
		}

		let index = match entity.blocks.iter().position(|block| block.material == material) {
			Some(index) => index,
			None => {
				entity.blocks.push(EntityBlock {
					material,
					range: 0..0,
					stale: true,
				});
				entity.blocks.len() - 1
			}
		};
		let block = &mut entity.blocks[index];
		if !block.stale && block.range.len() == length {
			return block.range.start;
		}
		// A reloaded material can change its parameter count, which needs a block of the new length.
		if block.range.len() != length {
			self.ranges.free(std::mem::replace(&mut block.range, 0..0));
			let Some(range) = self.ranges.allocate(length as u32) else {
				report(&mut self.reported, Warning::NoRoom);
				entity.blocks.swap_remove(index);
				return shared;
			};
			block.range = range;
		}
		block.stale = false;
		let range = block.range.clone();

		self.entries.copy(material_block.range.clone(), range.start);
		for (name, value) in &entity.values {
			match write(&mut self.entries, &material_block.parameters, range.clone(), name, *value) {
				Ok(()) => {}
				// Another material of the entity may declare it; only a name no loaded material declares is a mistake.
				Err(WriteError::Undeclared) => {
					let declared = self
						.materials
						.iter()
						.flatten()
						.any(|material| material.parameters.iter().any(|parameter| parameter.name == **name));
					if !declared {
						report(&mut self.reported, Warning::UndeclaredOnEntity { name });
					}
				}
				Err(error) => report(&mut self.reported, error.warning(&material_block.id, name, *value)),
			}
		}
		range.start
	}

	/// Frees the blocks of a renderable whose primitives leave the scene, keeping the values set on it.
	pub(crate) fn release_entity(&mut self, handle: Handle) {
		if let Some(entity) = self.entities.get_mut(&handle) {
			for block in entity.blocks.drain(..) {
				self.ranges.free(block.range);
			}
		}
	}

	/// Frees the blocks of a deleted renderable and forgets the values set on it.
	pub(crate) fn forget_entity(&mut self, handle: Handle) {
		self.release_entity(handle);
		self.entities.remove(&handle);
	}

	/// Returns the first changed entry and the changed entries the buffer of frame sequence `sequence` lacks, and
	/// records that it will hold them.
	pub(crate) fn take_changes(&mut self, sequence: usize) -> Option<(usize, &[[f32; 4]])> {
		let range = std::mem::replace(&mut self.entries.dirty[sequence], 0..0);
		let range = range.start as usize..range.end as usize;
		(!range.is_empty()).then(|| (range.start, &self.entries.values[range]))
	}

	/// Marks every entity block of the material at table slot `slot` for rewriting.
	fn mark_material_users_stale(&mut self, slot: u32) {
		for entity in self.entities.values_mut() {
			for block in entity.blocks.iter_mut().filter(|block| block.material == slot) {
				block.stale = true;
			}
		}
	}
}

/// The `Entries` struct keeps the buffer's values and which of them each frame sequence's copy lacks.
struct Entries {
	values: Box<[[f32; 4]]>,
	/// The entries changed since each frame sequence last copied them.
	dirty: [Range<u32>; ghi::MAX_FRAMES_IN_FLIGHT],
}

impl Entries {
	fn write(&mut self, entry: u32, value: bemx::Value) {
		let mut padded = [0.0; 4];
		padded[..value.components().len()].copy_from_slice(value.components());
		self.values[entry as usize] = padded;
		self.mark(entry..entry + 1);
	}

	fn copy(&mut self, source: Range<u32>, destination: u32) {
		let length = source.len() as u32;
		self.values
			.copy_within(source.start as usize..source.end as usize, destination as usize);
		self.mark(destination..destination + length);
	}

	/// Widens every frame sequence's changed range to cover `range`.
	fn mark(&mut self, range: Range<u32>) {
		for dirty in &mut self.dirty {
			*dirty = if dirty.start == dirty.end {
				range.clone()
			} else {
				dirty.start.min(range.start)..dirty.end.max(range.end)
			};
		}
	}
}

/// Records `value` for the parameter `name`, and returns whether that changed anything.
fn set(values: &mut Values, name: &Cow<'static, str>, value: bemx::Value) -> bool {
	match values.iter_mut().find(|(set, _)| set == name) {
		Some((_, set)) if *set == value => false,
		Some((_, set)) => {
			*set = value;
			true
		}
		None => {
			values.push((name.clone(), value));
			true
		}
	}
}

/// Writes `value` into the entry of the parameter `name` in a block at `range` laid out as `parameters`.
fn write(
	entries: &mut Entries,
	parameters: &[DynamicParameter],
	range: Range<u32>,
	name: &str,
	value: bemx::Value,
) -> Result<(), WriteError> {
	let (index, parameter) = parameters
		.iter()
		.enumerate()
		.find(|(_, parameter)| parameter.name == name)
		.ok_or(WriteError::Undeclared)?;
	if parameter.default.r#type() != value.r#type() {
		return Err(WriteError::Mismatch(parameter.default.r#type()));
	}
	// A block without room is empty; its allocation already warned.
	if let Some(entry) = range.clone().nth(index) {
		entries.write(entry, value);
	}
	Ok(())
}

/// The `WriteError` enum explains why a block can't take a value.
enum WriteError {
	/// The block's material doesn't declare the parameter dynamic.
	Undeclared,
	/// The material declares the parameter with this type.
	Mismatch(bemx::Type),
}

impl WriteError {
	fn warning<'a>(self, material: &'a str, name: &'a str, value: bemx::Value) -> Warning<'a> {
		match self {
			WriteError::Undeclared => Warning::Undeclared { material, name },
			WriteError::Mismatch(declared) => Warning::Mismatch {
				material,
				name,
				declared,
				set: value.r#type(),
			},
		}
	}
}

/// The `Warning` enum is one problem with the parameter values an application set.
#[derive(Hash)]
enum Warning<'a> {
	NoRoom,
	Undeclared {
		material: &'a str,
		name: &'a str,
	},
	UndeclaredOnEntity {
		name: &'a str,
	},
	Mismatch {
		material: &'a str,
		name: &'a str,
		declared: bemx::Type,
		set: bemx::Type,
	},
}

/// Logs `warning` the first time it happens.
fn report(reported: &mut HashSet<u64>, warning: Warning<'_>) {
	let mut hasher = FxHasher::default();
	warning.hash(&mut hasher);
	if !reported.insert(hasher.finish()) {
		return;
	}
	let undeclared_cause = "The most likely cause is a misspelt name, or a variable not marked `\"type\": \"Dynamic\"` in the material's `.bema` file.";
	match warning {
		Warning::NoRoom => warn!(
			"Visibility material parameters have no room left, so some instances read other values. The most likely cause is that loaded materials and entities with their own values need more than {MAX_MATERIAL_PARAMETERS} entries."
		),
		Warning::Undeclared { material, name } => {
			warn!("Visibility material {material} has no dynamic parameter '{name}'. {undeclared_cause}")
		}
		Warning::UndeclaredOnEntity { name } => warn!(
			"Visibility material parameter '{name}' was set on an entity, but no loaded material declares it dynamic. {undeclared_cause}"
		),
		Warning::Mismatch {
			material,
			name,
			declared,
			set,
		} => warn!(
			"Visibility material {material} declares its parameter '{name}' as a {}, but it was set to a {}. The most likely cause is a value of the wrong type.",
			declared.name(),
			set.name()
		),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn layout() -> Arc<[DynamicParameter]> {
		Arc::from([
			DynamicParameter {
				name: "tint".to_owned(),
				default: [1.0, 1.0, 1.0, 1.0].into(),
			},
			DynamicParameter {
				name: "glow".to_owned(),
				default: 0.0.into(),
			},
		])
	}

	/// Returns `length` entries from `start` of a buffer that received every change so far.
	fn uploaded(parameters: &mut MaterialParameters, start: u32, length: usize) -> Vec<[f32; 4]> {
		let mut buffer = vec![[f32::NAN; 4]; MAX_MATERIAL_PARAMETERS];
		if let Some((first, changes)) = parameters.take_changes(0) {
			buffer[first..first + changes.len()].copy_from_slice(changes);
		}
		buffer[start as usize..start as usize + length].to_vec()
	}

	fn handle(id: u32) -> Handle {
		Handle::from_id(id)
	}

	fn solid(parameters: &mut MaterialParameters) {
		parameters.adopt_material(0, Arc::from("solid.bema"), layout());
	}

	#[test]
	fn a_material_block_holds_its_defaults_and_the_values_set_before_it_loaded() {
		let mut parameters = MaterialParameters::new();
		parameters.receive(SetMaterialParameter::material("solid.bema", "glow", 2.0));
		solid(&mut parameters);

		let start = parameters.block(handle(1), 0);

		assert_eq!(
			uploaded(&mut parameters, start, 2),
			[[1.0, 1.0, 1.0, 1.0], [2.0, 0.0, 0.0, 0.0]]
		);
	}

	#[test]
	fn a_value_of_the_wrong_type_or_name_leaves_the_block_unchanged() {
		let mut parameters = MaterialParameters::new();
		solid(&mut parameters);
		parameters.receive(SetMaterialParameter::material("solid.bema", "tint", 3.0));
		parameters.receive(SetMaterialParameter::material("solid.bema", "missing", 3.0));

		let start = parameters.block(handle(1), 0);

		assert_eq!(
			uploaded(&mut parameters, start, 2),
			[[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]]
		);
	}

	#[test]
	fn an_entity_reads_its_own_block_once_a_value_is_set_on_it() {
		let mut parameters = MaterialParameters::new();
		solid(&mut parameters);
		let shared = parameters.block(handle(1), 0);

		parameters.receive(SetMaterialParameter::new(handle(1), "glow", 5.0));
		let own = parameters.block(handle(1), 0);

		assert_ne!(own, shared);
		assert_eq!(parameters.block(handle(2), 0), shared);
		assert_eq!(
			uploaded(&mut parameters, own, 2),
			[[1.0, 1.0, 1.0, 1.0], [5.0, 0.0, 0.0, 0.0]]
		);
	}

	#[test]
	fn an_entity_block_follows_material_changes_it_did_not_override() {
		let mut parameters = MaterialParameters::new();
		solid(&mut parameters);
		parameters.receive(SetMaterialParameter::new(handle(1), "glow", 5.0));
		parameters.block(handle(1), 0);

		parameters.receive(SetMaterialParameter::material("solid.bema", "tint", [0.0, 0.0, 1.0, 1.0]));
		parameters.receive(SetMaterialParameter::material("solid.bema", "glow", 7.0));
		let own = parameters.block(handle(1), 0);

		assert_eq!(
			uploaded(&mut parameters, own, 2),
			[[0.0, 0.0, 1.0, 1.0], [5.0, 0.0, 0.0, 0.0]]
		);
	}

	#[test]
	fn a_value_set_again_uploads_nothing() {
		let mut parameters = MaterialParameters::new();
		solid(&mut parameters);
		parameters.receive(SetMaterialParameter::new(handle(1), "glow", 5.0));
		parameters.block(handle(1), 0);
		parameters.take_changes(0);

		parameters.receive(SetMaterialParameter::new(handle(1), "glow", 5.0));
		parameters.block(handle(1), 0);

		assert_eq!(parameters.take_changes(0), None);
	}

	#[test]
	fn a_deleted_entity_gives_its_block_to_the_next_one() {
		let mut parameters = MaterialParameters::new();
		solid(&mut parameters);
		parameters.receive(SetMaterialParameter::new(handle(1), "glow", 5.0));
		let first = parameters.block(handle(1), 0);

		parameters.forget_entity(handle(1));
		parameters.receive(SetMaterialParameter::new(handle(2), "glow", 6.0));

		assert_eq!(parameters.block(handle(2), 0), first);
		assert_eq!(parameters.block(handle(1), 0), parameters.block(handle(3), 0));
	}
}
