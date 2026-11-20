//! A gym: a level the designer browses, in a large panel over the scene, to pick an asset to place in the level they
//! edit, as a museum shows its pieces.
//!
//! The Browse gym command lists the levels, and picking one spawns it on the editor's preview stage, apart from the
//! game's world, through [`Stage::open_level`](crate::editor::preview::Stage::open_level). The editor flies the gym's
//! camera with [`Fly`], asks a screen query where a click lands, and traces the entity it finds back to the gym
//! entity that placed it with [`Gym::pick`]. Enter, or the panel's Place button, carries what was picked into the
//! scene. See [Editor design](/docs/develop/editor#gyms).

use serde_json::Value;

use crate::core::factory::Handle;
use crate::editor::fly::Fly;
use crate::gameplay::Transform;
use crate::inspector::Spawned;
use crate::math::{Point, Scale, Vector, orientation_from_direction};

/// The share of the window's width and height left around the gym's panel, on each side.
const MARGIN: f32 = 0.1;
/// The space between the panel's edge and its caption and picture, and between the two.
pub(crate) const INSET: f32 = 8.0;
/// The height of the header over the picture, which holds the caption and the Place button. The button fills it, so
/// the panel's [`INSET`] alone spaces it from the edges and the picture.
pub(crate) const CAPTION: f32 = 30.0;
/// Where a gym's camera starts, from where the level stands, in meters: behind it and above, so it looks across the
/// level's floor.
const START: [f32; 3] = [0.0, 4.0, -12.0];
/// The point the camera starts looking at, from where the level stands, in meters.
const LOOK_AT: [f32; 3] = [0.0, 1.0, 0.0];

/// The `Gym` struct is the level being browsed for an asset: where it shows on the stage, how its camera flies, and
/// what the designer picked in it.
pub(crate) struct Gym {
	/// The level's source asset ID, such as `gyms/props.level`.
	pub(crate) id: String,
	/// The stage slot the gym shows in.
	pub(crate) slot: usize,
	/// The spawn, which traces an entity back to the level's source.
	spawned: Spawned,
	/// The level's entities as its source writes them, by their position in the file.
	entities: Vec<Value>,
	/// The camera's place and motion, which the fly keys and the right-button drag move.
	pub(crate) fly: Fly,
	/// The screen query asked for the entity under the last click, while it waits for its answer.
	pub(crate) picking: Option<Handle>,
	/// What the designer picked, which Enter or the Place button carries into the scene.
	pub(crate) picked: Option<Pick>,
}

impl Gym {
	/// The gym's file name, which the panel's caption shows.
	pub(crate) fn file(&self) -> &str {
		self.id.rsplit('/').next().unwrap_or(&self.id)
	}
}

/// The `Pick` struct is the asset a click in the gym picked, as the carry spawns it in the scene.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pick {
	/// What the caption calls it: the entity's name, or its asset's ID.
	pub(crate) name: String,
	pub(crate) asset: Asset,
}

/// The `Asset` enum is what a gym entity places, so the carry spawns the same thing in the scene.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Asset {
	/// A mesh, with its whole `mesh` payload: its source, its material, and its size.
	Mesh(Value),
	/// A prefab, by ID.
	Prefab(String),
}

impl Gym {
	/// The gym `id` that `spawned` placed on the stage's `slot`, from its parsed `source`, seen from `fly`.
	pub(crate) fn new(id: &str, slot: usize, spawned: Spawned, source: &Value, fly: Fly) -> Self {
		let entities = source.get("entities").and_then(Value::as_array).cloned().unwrap_or_default();
		Self {
			id: id.to_owned(),
			slot,
			spawned,
			entities,
			fly,
			picking: None,
			picked: None,
		}
	}

	/// Picks what the gym entity that placed `entity`, a handle a screen query found, holds; `None` when it's no
	/// entity of the gym, or places no mesh or prefab.
	pub(crate) fn pick(&self, entity: Handle) -> Option<Pick> {
		let offset = entity.id().checked_sub(self.spawned.root.id() + 1)?;
		let spawn = self.spawned.loaded.placer(u32::try_from(offset).ok()?)?;
		let source = self.entities.get(*self.spawned.order.get(spawn as usize)?)?;
		asset(source)
	}
}

/// Where a gym's camera starts over a level standing at `origin`: behind it and above, looking across its floor.
pub(crate) fn start(origin: Point) -> Transform {
	let [x, y, z] = START;
	let [a, b, c] = LOOK_AT;
	let position = origin + Vector::new(x, y, z);
	let direction = (origin + Vector::new(a, b, c) - position)
		.normalized()
		.expect("The camera starts away from the point it looks at.");
	Transform::new(position, Scale::identity(), orientation_from_direction(direction))
}

/// What one entity of a level's source places: its prefab, or else its mesh. It's named by its `name` component, its
/// `id`, or else what it places: the asset's ID, or a generated mesh's shape, such as `box`.
fn asset(entity: &Value) -> Option<Pick> {
	let (asset, placed) = if let Some(prefab) = entity.pointer("/prefab/resource").and_then(Value::as_str) {
		(Asset::Prefab(prefab.to_owned()), prefab)
	} else {
		let mesh = entity.pointer("/components/mesh")?;
		let resource = mesh.get("resource").and_then(Value::as_str);
		let shape = mesh.as_object().and_then(|source| source.keys().next()).map(String::as_str);
		(Asset::Mesh(mesh.clone()), resource.or(shape).unwrap_or("mesh"))
	};
	let name = entity
		.pointer("/components/name")
		.or_else(|| entity.get("id"))
		.and_then(Value::as_str)
		.unwrap_or(placed);
	Some(Pick {
		name: name.to_owned(),
		asset,
	})
}

/// Where the gym's panel sits in a window of `viewport` pixels, `[width, height]`, as `[x, y, width, height]` from the
/// window's top-left corner: centered, with [`MARGIN`] of the window around it.
pub(crate) fn panel(viewport: [f32; 2]) -> [f32; 4] {
	let [width, height] = viewport;
	[
		width * MARGIN,
		height * MARGIN,
		width * (1.0 - 2.0 * MARGIN),
		height * (1.0 - 2.0 * MARGIN),
	]
}

/// Where the gym's picture sits in a window of `viewport` pixels, as [`panel`] gives it: under the caption, inside
/// the panel's inset, at least a pixel each way.
pub(crate) fn picture(viewport: [f32; 2]) -> [f32; 4] {
	let [x, y, width, height] = panel(viewport);
	let top = INSET + CAPTION + INSET;
	[
		x + INSET,
		y + top,
		(width - 2.0 * INSET).max(1.0),
		(height - top - INSET).max(1.0),
	]
}

/// The pixel of the gym's picture under a pointer at `pointer`, in normalized window coordinates from -1 at the left
/// and bottom edges to 1 at the right and top, in a window of `viewport` pixels; `None` outside the picture.
pub(crate) fn picture_pixel(pointer: [f32; 2], viewport: [f32; 2]) -> Option<[u32; 2]> {
	let [x, y, width, height] = picture(viewport);
	let window = [(pointer[0] + 1.0) * 0.5 * viewport[0], (1.0 - pointer[1]) * 0.5 * viewport[1]];
	let inside = [window[0] - x, window[1] - y];
	(inside[0] >= 0.0 && inside[1] >= 0.0 && inside[0] < width && inside[1] < height)
		.then(|| [inside[0] as u32, inside[1] as u32])
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn an_entity_places_its_prefab_or_its_whole_mesh_by_its_name() {
		let crate_mesh = json!({ "resource": "props/crate.glb", "material": "wood.bema" });
		let mesh = asset(&json!({ "components": { "name": "Crate", "mesh": crate_mesh } }));
		assert_eq!(
			mesh,
			Some(Pick {
				name: "Crate".into(),
				asset: Asset::Mesh(crate_mesh),
			})
		);
		let prefab = asset(&json!({ "prefab": { "resource": "lamp.prefab" }, "components": { "mesh": { "sphere": {} } } }));
		assert_eq!(
			prefab.map(|pick| (pick.name, pick.asset)),
			Some(("lamp.prefab".into(), Asset::Prefab("lamp.prefab".into())))
		);
		let shape = asset(&json!({ "components": { "mesh": { "box": { "half_extents": [1, 1, 1] } } } }));
		assert_eq!(
			shape.map(|pick| pick.name),
			Some("box".into()),
			"an unnamed generated mesh goes by its shape"
		);
		let block = asset(&json!({ "id": "block", "components": { "mesh": { "box": {} } } }));
		assert_eq!(
			block.map(|pick| pick.name),
			Some("block".into()),
			"an entity without a name goes by its id"
		);
		assert_eq!(
			asset(&json!({ "components": { "point_light": {} } })),
			None,
			"a light has nothing to place"
		);
	}

	#[test]
	fn a_pointer_over_the_picture_names_its_pixel_and_one_outside_names_none() {
		let viewport = [1000.0, 800.0];
		let [x, y, ..] = picture(viewport);
		// The pointer half a pixel inside the picture's top-left corner, in normalized window coordinates.
		let corner = [(x + 0.5) / viewport[0] * 2.0 - 1.0, 1.0 - (y + 0.5) / viewport[1] * 2.0];
		assert_eq!(picture_pixel(corner, viewport), Some([0, 0]));
		assert_eq!(
			picture_pixel([-1.0, 1.0], viewport),
			None,
			"the window's corner is outside the panel"
		);
	}
}
