//! The icons the editor draws, by the purpose each one serves, so a game can swap any of them for its own.
//!
//! Give [`crate::editor::install`] a list of [`Vector`] images named by the constants in this module. Each name is a place in
//! the editor, not a picture: [`DELETE_ENTITY`] is the button that deletes the selected entity, whatever it shows. Two
//! places that share a picture today still take one icon each, so a game can tell them apart later.

use crate::ui::{CurvePoint, CurveSegment, FillRule, Vector, VectorLayer};

/// The `Icons` type names the icons the editor draws, as pairs of a purpose from this module and the [`Vector`] to
/// draw there, such as one [`crate::ui::load_vector`] loads from an SVG asset. Pass it to [`crate::editor::install`];
/// a purpose left out keeps the editor's own icon.
pub type Icons<'a> = &'a [(&'static str, Vector)];

/// The button in the scene panel that deletes the selected entity.
pub const DELETE_ENTITY: &str = "delete-entity";
/// The button that dismisses a toast.
pub const CLOSE_TOAST: &str = "close-toast";
/// The button on a graph node that removes the node.
pub const REMOVE_NODE: &str = "remove-node";
/// The badge of a toast that reports a note.
pub const TOAST_NOTE: &str = "toast-note";
/// The badge of a toast that reports a warning.
pub const TOAST_WARNING: &str = "toast-warning";
/// The badge of a toast that reports an error.
pub const TOAST_ERROR: &str = "toast-error";
/// The mark on each entity's row in the scene list.
pub const ENTITY: &str = "entity";
/// The palette entry that places a mesh.
pub const PLACE_MESH: &str = "place-mesh";
/// The palette entry that places a prefab.
pub const PLACE_PREFAB: &str = "place-prefab";
/// The palette entry that places a point light, which shines every way from one point.
pub const PLACE_POINT_LIGHT: &str = "place-point-light";
/// The palette entry that places a cone light, which shines one way in a cone.
pub const PLACE_CONE_LIGHT: &str = "place-cone-light";
/// The palette entry that places a directional light, which shines one way everywhere, as the sun does.
pub const PLACE_DIRECTIONAL_LIGHT: &str = "place-directional-light";
/// The palette entry that opens a material graph.
pub const OPEN_GRAPH: &str = "open-graph";
/// The palette entry that previews a texture.
pub const PREVIEW_TEXTURE: &str = "preview-texture";
/// The palette entry that previews a stored mesh.
pub const PREVIEW_MESH: &str = "preview-mesh";
/// The palette entry that opens a material variant.
pub const OPEN_VARIANT: &str = "open-variant";
/// The palette entry that adds a node to the open graph.
pub const ADD_NODE: &str = "add-node";
/// The palette entry that shows a runtime setting.
pub const OPEN_SETTING: &str = "open-setting";
/// The palette entry that starts a new level.
pub const NEW_LEVEL: &str = "new-level";
/// The palette entry that starts a new material.
pub const NEW_MATERIAL: &str = "new-material";
/// The palette entry that starts a new material variant.
pub const NEW_VARIANT: &str = "new-variant";
/// The palette entry that picks the root material a new variant specializes.
pub const PICK_ROOT_MATERIAL: &str = "pick-root-material";
/// The palette entry that picks the folder what's being created goes in.
pub const PICK_FOLDER: &str = "pick-folder";
/// The palette entry that lists the levels to browse as a gym.
pub const BROWSE_GYMS: &str = "browse-gyms";
/// The palette entry that opens a level as a gym.
pub const OPEN_GYM: &str = "open-gym";
/// The palette entry that moves the camera to a waypoint.
pub const GO_TO_WAYPOINT: &str = "go-to-waypoint";
/// The palette entry that saves a waypoint where the camera stands.
pub const SAVE_WAYPOINT: &str = "save-waypoint";
/// The palette entry that finds an entity of the scene and turns the camera onto it.
pub const FIND_ENTITY: &str = "find-entity";

/// Every purpose the editor draws an icon for.
pub const PURPOSES: [&str; 28] = [
	DELETE_ENTITY,
	CLOSE_TOAST,
	REMOVE_NODE,
	TOAST_NOTE,
	TOAST_WARNING,
	TOAST_ERROR,
	ENTITY,
	PLACE_MESH,
	PLACE_PREFAB,
	PLACE_POINT_LIGHT,
	PLACE_CONE_LIGHT,
	PLACE_DIRECTIONAL_LIGHT,
	OPEN_GRAPH,
	PREVIEW_TEXTURE,
	PREVIEW_MESH,
	OPEN_VARIANT,
	ADD_NODE,
	OPEN_SETTING,
	NEW_LEVEL,
	NEW_MATERIAL,
	NEW_VARIANT,
	PICK_ROOT_MATERIAL,
	PICK_FOLDER,
	BROWSE_GYMS,
	OPEN_GYM,
	GO_TO_WAYPOINT,
	SAVE_WAYPOINT,
	FIND_ENTITY,
];

/// The icons the editor and its windows draw: the editor's own vectors, each replaced by the game's icon for the same
/// purpose, and the game's icons for places that otherwise draw a tile. Logs each name that isn't a purpose, since the
/// editor never draws it.
pub(crate) fn merged(icons: Icons<'_>) -> Box<[(&'static str, Vector)]> {
	for (name, _) in icons {
		if !PURPOSES.contains(name) {
			log::warn!(
				"The editor doesn't draw an icon named \"{name}\". The name is likely misspelled or names a picture instead of a place. Name it by one of the purposes in byte_engine::editor::icons, such as DELETE_ENTITY."
			);
		}
	}
	// The marks cut out of the toasts' badges: an i, and an exclamation point.
	let info = [rectangle(7.1, 3.8, 8.9, 5.6), rectangle(7.1, 6.8, 8.9, 12.2)];
	let exclamation = [rectangle(7.1, 3.6, 8.9, 9.6), rectangle(7.1, 10.8, 8.9, 12.6)];
	let defaults = [
		(DELETE_ENTITY, trash_can()),
		(CLOSE_TOAST, cross()),
		(REMOVE_NODE, cross()),
		(TOAST_NOTE, badge(&info)),
		(TOAST_WARNING, badge(&exclamation)),
		(TOAST_ERROR, badge(&exclamation)),
	];
	let kept = defaults
		.into_iter()
		.filter(|(purpose, _)| !icons.iter().any(|(name, _)| name == purpose));
	icons.iter().cloned().chain(kept).collect()
}

/// The icon for `purpose` among `icons`, or `None` when the place draws its own tile.
pub(crate) fn find<'a>(icons: &'a [(&'static str, Vector)], purpose: &str) -> Option<&'a Vector> {
	icons.iter().find(|(name, _)| *name == purpose).map(|(_, icon)| icon)
}

/// The corners of a rectangle, clockwise from its top left.
fn rectangle(left: f32, top: f32, right: f32, bottom: f32) -> [(f32, f32); 4] {
	[(left, top), (right, top), (right, bottom), (left, bottom)]
}

/// Appends `points` to `segments` as one closed contour of lines.
fn contour(segments: &mut Vec<CurveSegment>, points: &[(f32, f32)]) {
	for (index, &from) in points.iter().enumerate() {
		segments.push(CurveSegment::Line {
			from: from.into(),
			to: points[(index + 1) % points.len()].into(),
		});
	}
}

/// A trash can in a 16 by 16 box: a lid with a handle over a hollow can with two slats. Its contours fill by the
/// even-odd rule, so the can's inner outline cuts it hollow.
fn trash_can() -> Vector {
	let mut segments = Vec::new();
	// The lid and its handle.
	contour(&mut segments, &rectangle(2.0, 3.0, 14.0, 4.5));
	contour(&mut segments, &rectangle(6.0, 1.25, 10.0, 3.0));
	// The can, tapering toward its foot, and the hollow inside it.
	contour(&mut segments, &[(3.0, 5.5), (13.0, 5.5), (12.2, 15.0), (3.8, 15.0)]);
	contour(&mut segments, &[(4.5, 7.0), (11.5, 7.0), (10.9, 13.5), (5.1, 13.5)]);
	// The slats, inside the hollow.
	contour(&mut segments, &rectangle(6.4, 8.5, 7.3, 12.0));
	contour(&mut segments, &rectangle(8.7, 8.5, 9.6, 12.0));
	Vector::new([16.0, 16.0], vec![VectorLayer::plain(segments, FillRule::EvenOdd)])
}

/// An × in a 16 by 16 box: two crossed bars, which fill as one by the non-zero rule since both wind the same way.
fn cross() -> Vector {
	// Half a bar's thickness, across the bar's diagonal.
	const HALF: f32 = 0.75;
	let mut segments = Vec::new();
	let (low, high) = (3.5, 12.5);
	contour(
		&mut segments,
		&[
			(low + HALF, low - HALF),
			(high + HALF, high - HALF),
			(high - HALF, high + HALF),
			(low - HALF, low + HALF),
		],
	);
	contour(
		&mut segments,
		&[
			(high + HALF, low + HALF),
			(low + HALF, high + HALF),
			(low - HALF, high - HALF),
			(high - HALF, low - HALF),
		],
	);
	Vector::new([16.0, 16.0], vec![VectorLayer::plain(segments, FillRule::NonZero)])
}

/// A disc in a 16 by 16 box with `marks` cut out of it by the even-odd rule, so the card shows through them.
fn badge(marks: &[[(f32, f32); 4]]) -> Vector {
	// The distance of a quarter circle's control points from its ends, as a fraction of the radius, which keeps four
	// cubics within 0.03 % of a circle.
	const KAPPA: f32 = 0.552_284_8;
	let (center, radius) = (8.0, 8.0);
	let reach = radius * KAPPA;
	let point = |x: f32, y: f32| CurvePoint::new(center + x, center + y);
	let mut segments: Vec<CurveSegment> = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)]
		.iter()
		.zip([(0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (1.0, 0.0)])
		.map(|(&(x, y), (nx, ny))| CurveSegment::Cubic {
			from: point(x * radius, y * radius),
			control0: point(x * radius - y * reach, y * radius + x * reach),
			control1: point(nx * radius + ny * reach, ny * radius - nx * reach),
			to: point(nx * radius, ny * radius),
		})
		.collect();
	for mark in marks {
		contour(&mut segments, mark);
	}
	Vector::new([16.0, 16.0], vec![VectorLayer::plain(segments, FillRule::EvenOdd)])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_games_icon_replaces_the_editors_own_for_its_purpose() {
		let own = Vector::new([24.0, 24.0], Vec::new());
		let icons = merged(&[(DELETE_ENTITY, own.clone()), (PLACE_MESH, own.clone())]);
		assert!(find(&icons, DELETE_ENTITY) == Some(&own));
		assert!(find(&icons, PLACE_MESH) == Some(&own));
		// Places the game left alone keep the editor's own icon, and tiles stay tiles.
		assert!(find(&icons, CLOSE_TOAST) == Some(&cross()));
		assert!(find(&icons, PLACE_PREFAB).is_none());
	}
}
