//! The stage the editor's previews draw on: a scene of its own, apart from the game's world, where each preview is a
//! subject, such as a mesh or a sphere wearing a material, which a drag turns as an arcball, seen by a camera that a
//! scroll moves closer.
//!
//! [`Stage::install`] creates the scene and its visibility pipeline while the game sets up. A resource window that
//! previews something opens a slot with [`Stage::open`], shows [`Stage::image`] in its main panel, turns and zooms it
//! with [`Stage::steer`], and frees the slot with [`Stage::close`] when it closes. Render images are never freed, so a
//! closed slot is kept, shrunk to a pixel, for the next preview. See
//! [render images](/docs/reference/rendering#render-images).

use utils::Extent;

use crate::application::graphics::{GraphicsApplication, setup_pbr_visibility_shading_render_pipeline};
use crate::constants::up;
use crate::core::Creator as _;
use crate::core::channel::{Channel as _, DefaultChannel};
use crate::core::factory::Handle;
use crate::core::factory::CreateMessage;
use crate::core::listener::{DefaultListener, Listener as _};
use crate::core::message::DeleteMessage;
use crate::core::message_bus::MessageScope;
use crate::editor::fly::heading;
use crate::gameplay::Transform;
use crate::gameplay::level::Placement;
use crate::inspector::Spawned;
use crate::gameplay::transform::TransformationUpdate;
use crate::math::{Orientation, Point, Scale, UnitVector, Vector, orientation_from_direction};
use crate::rendering::mesh::generator::SphereMeshGenerator;
use crate::rendering::{
	Camera, DirectionalLight, LightColor, PhotometricIntensity, RenderImage, RenderImageReady, RenderableMesh, SceneId,
	ScreenSceneHit, ScreenSceneQuery, SetRenderImageExtent,
};

/// How far apart slots stand, in meters, so no preview's camera reaches another's subject.
const SPACING: f32 = 10_000.0;
/// The farthest a level's camera sees, in meters, short of the next slot.
const LEVEL_FAR: f32 = 2_000.0;
/// The radius of the sphere a material is shown on, in meters.
const SPHERE_RADIUS: f32 = 0.5;
/// How many radii of its subject a camera stands from it when a preview opens, which fits the subject in the default
/// field of view with a margin.
const FRAMING: f32 = 3.0;
/// The nearest and farthest a camera can come, in radii of its subject.
const ZOOM_RANGE: [f32; 2] = [1.2, 20.0];
/// How much one scroll step moves the camera closer, as a share of its distance.
const ZOOM_STEP: f32 = 0.9;
/// The heading a preview's camera looks along: from in front of the subject, a little to the side and above it, as a
/// turn about the vertical from +Z toward +X and a tilt above the horizon, in radians.
const VIEW: [f32; 2] = [std::f32::consts::PI + 0.6, -0.35];
/// How brightly the stage's sun lights it, in lux: a bright overcast day.
const SUN_ILLUMINANCE: f32 = 20_000.0;

/// The exposure, in stops, that shows a surface the stage's sun lights at mid-gray: the incident-light meter equation,
/// EV100 = log2(E·S/C) with ISO S = 100 and calibration constant C = 250, turned into stops as
/// [`Camera::set_exposure`] says.
fn stage_exposure() -> f32 {
	let ev100 = (SUN_ILLUMINANCE * 100.0 / 250.0).log2();
	-(ev100 + 1.2f32.log2())
}

/// The `Subject` enum is what a preview shows, so a resource window can ask for its own.
pub(crate) enum Subject<'a> {
	/// A sphere drawn with the material resource this names, such as a variant's `.bema` file.
	Material(&'a str),
	/// The stored mesh this names, with the box that holds it in its own coordinates.
	Mesh(&'a str, crate::math::AABB<resource_management::resources::ModelSpace>),
}

/// The point on Shoemake's arcball under a pointer at `point`, in coordinates where the ball's center is the origin,
/// its radius 1, +x right, and +y up: on the ball's front half inside it, and on its rim outside it, as `[x, y, z]`
/// with +z toward the viewer.
fn ball_point([x, y]: [f32; 2]) -> [f32; 3] {
	let squared = x * x + y * y;
	if squared <= 1.0 {
		[x, y, (1.0 - squared).sqrt()]
	} else {
		let length = squared.sqrt();
		[x / length, y / length, 0.0]
	}
}

/// The `Framing` struct is how a preview shows its subject: the subject's turn, which a drag sets as an arcball, and
/// the distance of the camera that looks at it, which a scroll sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Framing {
	/// The point the camera looks at, where the subject's center stays as it turns.
	target: Point,
	/// The subject's center in its own coordinates, which turns about the target.
	center: Vector,
	/// The radius of a sphere that holds the subject, in meters, which sets how close the camera can come.
	radius: f32,
	/// The half extents of the box that holds the subject, in its own coordinates, which turns with it.
	half_extents: Vector,
	/// The camera's distance from the target, in meters.
	distance: f32,
	/// How the subject is turned.
	spin: Orientation,
	/// How far right of the picture's middle the subject shows, in pixels, and the picture's height in pixels, which
	/// sets how far the camera moves sideways to show it there.
	offset: f32,
	height: f32,
}

impl Framing {
	/// Frames a subject whose center, `center` in its own coordinates, stands at `target`, held by a sphere of `radius`
	/// and by a box of `half_extents` around its center.
	pub(crate) fn new(target: Point, center: Vector, radius: f32, half_extents: Vector) -> Self {
		let radius = radius.max(0.01);
		Self {
			target,
			center,
			radius,
			half_extents,
			distance: radius * FRAMING,
			spin: Orientation::default(),
			offset: 0.0,
			height: 1.0,
		}
	}

	/// The tangent of half the camera's vertical field of view, which relates distances in the world to pixels.
	fn half_view() -> f32 {
		(Camera::new().vertical_fov().to_radians().value() * 0.5).tan()
	}

	/// At most how wide the subject shows in a picture `height` pixels high: the narrower of the sphere and the turned
	/// box that hold it, each seen in perspective from the camera.
	pub(crate) fn width(&self, height: f32) -> f32 {
		self.sphere_width(height).min(self.box_width(height))
	}

	/// How wide the sphere that holds the subject shows in a picture `height` pixels high.
	fn sphere_width(&self, height: f32) -> f32 {
		// A sphere of radius `r` seen from `d` away spans an angle whose tangent is `r / √(d² - r²)` either side.
		let reach = self.radius / (self.distance * self.distance - self.radius * self.radius).max(1e-6).sqrt();
		reach / Self::half_view() * height
	}

	/// How wide the box that holds the subject, turned with it, shows in a picture `height` pixels high: the span of its
	/// corners projected through the camera. A box the camera stands in shows wider than any picture.
	fn box_width(&self, height: f32) -> f32 {
		let [forward, right, _] = Self::screen_axes();
		let h = self.half_extents;
		// Pixels per unit of the tangent from the view's middle.
		let scale = height * 0.5 / Self::half_view();
		let mut span = [f32::INFINITY, f32::NEG_INFINITY];
		for corner in 0..8 {
			let sign = |bit: u32| if corner >> bit & 1 == 0 { -1.0 } else { 1.0 };
			let local = Vector::new(h.x() * sign(0), h.y() * sign(1), h.z() * sign(2));
			// The corner from the target, then from the camera, which stands `distance` back along the view.
			let placed = self.spin.rotate_vector(local);
			let depth = self.distance + placed.dot(forward);
			if depth <= 0.0 {
				return f32::INFINITY;
			}
			let x = placed.dot(right) / depth * scale;
			span = [span[0].min(x), span[1].max(x)];
		}
		span[1] - span[0]
	}

	/// Shows the subject `offset` pixels right of the middle of a picture `height` pixels high, by moving the camera
	/// sideways, so it stays centered in the room a drawer leaves.
	pub(crate) fn shift(&mut self, offset: f32, height: f32) {
		self.offset = offset;
		self.height = height.max(1.0);
	}

	/// The camera's view direction, right, and up in the world, as the view it sees lays them on the screen.
	fn screen_axes() -> [Vector; 3] {
		let forward = heading(VIEW[0], VIEW[1]).into_vector();
		// Up across a view that tilts is shorter than a unit, so it's scaled back to one.
		let right = up()
			.cross(forward)
			.normalized()
			.expect("The preview's view never looks straight up or down.")
			.into_vector();
		[forward, right, forward.cross(right)]
	}

	/// Turns the subject by a drag from `from` to `to`, both in arcball coordinates: Shoemake's arcball, which turns by
	/// twice the angle between the ball points under them, about the axis across both. A drag across the ball turns
	/// the subject the way it moves, and a drag around the rim rolls it.
	pub(crate) fn turn(&mut self, from: [f32; 2], to: [f32; 2]) {
		let [forward, right, screen_up] = Self::screen_axes();
		// The ball's +z faces the viewer, which is against the camera's view.
		let world = |[x, y, z]: [f32; 3]| right * x + screen_up * y - forward * z;
		let (from, to) = (world(ball_point(from)), world(ball_point(to)));
		let axis = from.cross(to);
		// The quaternion of a dot and a cross of unit vectors turns by twice their angle.
		let Ok(turn) = Orientation::try_from_array([axis.x(), axis.y(), axis.z(), from.dot(to)]) else {
			return;
		};
		self.spin = turn.compose(self.spin);
	}

	/// Moves the camera `steps` scroll steps closer, or farther for negative steps, within [`ZOOM_RANGE`].
	pub(crate) fn zoom(&mut self, steps: f32) {
		let [nearest, farthest] = ZOOM_RANGE.map(|radii| radii * self.radius);
		self.distance = (self.distance * ZOOM_STEP.powf(steps)).clamp(nearest, farthest);
	}

	/// Where the camera stands, looking along its heading past the target, moved sideways by the shift.
	pub(crate) fn camera_transform(&self) -> Transform {
		let view: UnitVector = heading(VIEW[0], VIEW[1]);
		let [_, right, _] = Self::screen_axes();
		// A pixel at the target's distance spans this many meters, and the camera moves against the shift.
		let meters_per_pixel = 2.0 * self.distance * Self::half_view() / self.height;
		let position = self.target - view.into_vector() * self.distance - right * (self.offset * meters_per_pixel);
		Transform::new(position, Scale::identity(), view.into())
	}

	/// Where the subject stands: turned by the spin, with its center kept at the target.
	pub(crate) fn subject_transform(&self) -> Transform {
		let position = self.target - self.spin.rotate_vector(self.center);
		Transform::new(position, Scale::identity(), self.spin)
	}

	/// The camera that sees the whole subject from as far as it goes, and nothing much farther.
	fn camera(&self) -> Camera {
		let far = self.radius * (ZOOM_RANGE[1] + 2.0);
		Camera::new()
			.with_clip_planes((self.radius * 0.01).max(0.001), far)
			.with_exposure(stage_exposure())
	}
}

/// The `Slot` struct is one preview on the stage: its render image, the camera that draws it, and what it shows.
struct Slot {
	render_image: Handle,
	camera: Handle,
	/// The image the renderer draws into, once it answered.
	image: Option<crate::ghi::ImageHandle>,
	/// The subject's entity, while a preview uses the slot.
	subject: Option<Handle>,
	framing: Framing,
	extent: Extent,
	/// The detail level the slot's camera draws its subject at, or `None` to select levels by distance.
	detail_level: Option<u32>,
}

/// The `Stage` struct is the editor's preview scene and the slots its resource windows show, kept for the whole run
/// since a scene's pipelines and render images can't be removed.
pub(crate) struct Stage {
	scene: SceneId,
	/// Where the stage's objects are created, apart from the world's.
	scope: MessageScope,
	slots: Vec<Slot>,
	ready: DefaultListener<RenderImageReady>,
	extents: DefaultChannel<SetRenderImageExtent>,
	/// The answers to screen queries about the stage's pictures, which the stage's own pipeline gives on its scope.
	hits: DefaultListener<CreateMessage<ScreenSceneHit>>,
}

impl Stage {
	/// Creates the stage's scene, its visibility pipeline, and its sun. Call it before the loading thread starts,
	/// since the pipeline loads on it.
	pub(crate) fn install(app: &mut GraphicsApplication) -> Self {
		let scene = app.create_scene("editor preview");
		setup_pbr_visibility_shading_render_pipeline(app, scene);
		let scope = app.scene_messages(scene).clone();
		let sun = DirectionalLight::new(LightColor::kelvin(6000.0), PhotometricIntensity::illuminance(SUN_ILLUMINANCE))
			.expect("The preview's sun is a valid light.");
		// A key light from above the view, a little to its left, so the side the camera sees is lit.
		let down: UnitVector = Vector::new(-0.2, -1.0, -0.7)
			.normalized()
			.expect("The sun's direction has length.");
		scope.create(sun).with(Transform::new(
			Point::origin(),
			Scale::identity(),
			orientation_from_direction(down),
		));
		Self {
			scene,
			ready: app.messages().channel::<RenderImageReady>().listener(),
			extents: app.messages().channel(),
			hits: scope.factory::<ScreenSceneHit>().listener(),
			scope,
			slots: Vec::new(),
		}
	}

	/// Shows `subject` in a slot `extent` pixels large, reusing a free one before opening another, and returns it.
	pub(crate) fn open(&mut self, app: &GraphicsApplication, subject: Subject, extent: [u32; 2]) -> usize {
		let index = self.free_slot();
		let origin = Self::origin(index);
		let (mesh, center, radius, half_extents) = match subject {
			Subject::Material(material) => (
				RenderableMesh::generated(Box::new(
					SphereMeshGenerator::from_radius(SPHERE_RADIUS).with_material(material),
				)),
				Vector::new(0.0, 0.0, 0.0),
				SPHERE_RADIUS,
				Vector::new(SPHERE_RADIUS, SPHERE_RADIUS, SPHERE_RADIUS),
			),
			Subject::Mesh(id, bounds) => {
				let (center, half_extents) = (bounds.center(), bounds.half_extents());
				(
					RenderableMesh::resource(id),
					Vector::new(center.x(), center.y(), center.z()),
					half_extents.length(),
					Vector::new(half_extents.x(), half_extents.y(), half_extents.z()),
				)
			}
		};
		let framing = Framing::new(origin + center, center, radius, half_extents);
		let subject = Some(self.scope.create(mesh).with(framing.subject_transform()).handle());
		self.fill(app, index, subject, framing);
		self.scope
			.factory::<Camera>()
			.derive(self.slots[index].camera, framing.camera());
		self.place(index);
		self.resize(index, extent);
		index
	}

	/// Spawns the level whose source is `source` on a slot `extent` pixels large, at the slot's spot, apart from the
	/// game's world, and returns the slot, the spawn, and where the level stands. Its camera stays where
	/// [`Self::view`] puts it; [`Self::close`] deletes the level.
	pub(crate) fn open_level(
		&mut self,
		app: &GraphicsApplication,
		source: &str,
		extent: [u32; 2],
	) -> Result<(usize, Spawned, Point), String> {
		let index = self.free_slot();
		let origin = Self::origin(index);
		let spawned = app.inspector().spawn_level_in(
			&self.scope,
			source,
			Placement::at(Transform::from_position(origin)),
		)?;
		// The framing only keeps the slot's shape; the level never turns, and the camera flies.
		let framing = Framing::new(origin, Vector::new(0.0, 0.0, 0.0), 1.0, Vector::new(1.0, 1.0, 1.0));
		self.fill(app, index, Some(spawned.root), framing);
		let camera = Camera::new()
			.with_clip_planes(0.1, LEVEL_FAR)
			.with_exposure(stage_exposure());
		self.scope.factory::<Camera>().derive(self.slots[index].camera, camera);
		self.resize(index, extent);
		Ok((index, spawned, origin))
	}

	/// The first slot no preview uses, or the next new one.
	fn free_slot(&self) -> usize {
		self.slots
			.iter()
			.position(|slot| slot.subject.is_none())
			.unwrap_or(self.slots.len())
	}

	/// Where the slot at `index` stands: each on its own spot, far from the others.
	fn origin(index: usize) -> Point {
		Point::new(index as f32 * SPACING, 0.0, 0.0)
	}

	/// Gives the slot at `index` its `subject` and `framing`, opening it with its camera and render image when it's new.
	fn fill(&mut self, app: &GraphicsApplication, index: usize, subject: Option<Handle>, framing: Framing) {
		if index == self.slots.len() {
			let camera = self.scope.create(Camera::new()).handle();
			let extent = Extent::rectangle(1, 1);
			let render_image = app
				.render_image_factory()
				.create(RenderImage::new(self.scene, extent, camera));
			self.slots.push(Slot {
				render_image,
				camera,
				image: None,
				subject,
				framing,
				extent,
				detail_level: None,
			});
		} else {
			let slot = &mut self.slots[index];
			slot.subject = subject;
			slot.framing = framing;
			slot.detail_level = None;
		}
	}

	/// Moves the slot's camera to `transform`, as the editor flies a gym's.
	pub(crate) fn view(&self, index: usize, transform: Transform) {
		let camera = self.slots[index].camera;
		self.scope
			.channel::<TransformationUpdate>()
			.send(TransformationUpdate::new(camera, transform));
	}

	/// Asks which entity the slot's picture shows at `pixel`, from its top-left pixel, and returns the query, which
	/// [`Self::answered`] answers a frame or two later.
	pub(crate) fn ask(&self, index: usize, pixel: [u32; 2]) -> Handle {
		let camera = self.slots[index].camera;
		self.scope.create(ScreenSceneQuery::new(camera, pixel)).handle()
	}

	/// The answers to queries [`Self::ask`] asked that arrived since the last call, as each query with the entity it
	/// found, if any.
	pub(crate) fn answered(&mut self) -> impl Iterator<Item = (Handle, Option<Handle>)> + '_ {
		std::iter::from_fn(|| {
			let message = self.hits.read()?;
			let hit = message.data();
			Some((hit.query(), hit.entity()))
		})
	}

	/// Removes the slot's subject and shrinks its image to a pixel, which keeps it for the next preview.
	pub(crate) fn close(&mut self, index: usize) {
		if let Some(subject) = self.slots[index].subject.take() {
			self.scope.channel::<DeleteMessage>().send(DeleteMessage::new(subject));
		}
		self.resize(index, [1, 1]);
	}

	/// Learns the images the renderer draws the slots into.
	pub(crate) fn adopt(&mut self) {
		while let Some(ready) = self.ready.read() {
			if let Some(slot) = self.slots.iter_mut().find(|slot| slot.render_image == ready.render_image()) {
				slot.image = Some(ready.image());
			}
		}
	}

	/// The image the slot is drawn into, once the renderer answered.
	pub(crate) fn image(&self, index: usize) -> Option<crate::ghi::ImageHandle> {
		self.slots[index].image
	}

	/// Draws the slot's subject at detail `level`, or by distance for `None`, by giving its camera that level; see
	/// [`Camera::with_detail_level`].
	pub(crate) fn show_level(&mut self, index: usize, level: Option<u32>) {
		let slot = &mut self.slots[index];
		if slot.detail_level == level {
			return;
		}
		slot.detail_level = level;
		let camera = slot.framing.camera().with_detail_level(level);
		self.scope.factory::<Camera>().derive(slot.camera, camera);
	}

	/// Turns the slot's subject by the arcball drag `turn`, from and to, and moves its camera `zoom` scroll steps
	/// closer; see [`Framing::turn`] and [`Framing::zoom`].
	pub(crate) fn steer(&mut self, index: usize, turn: Option<([f32; 2], [f32; 2])>, zoom: f32) {
		if turn.is_none() && zoom == 0.0 {
			return;
		}
		let framing = &mut self.slots[index].framing;
		if let Some((from, to)) = turn {
			framing.turn(from, to);
		}
		framing.zoom(zoom);
		self.place(index);
	}

	/// Fills a window of `viewport` layout units, `[width, height]`, with the slot's image, drawn at `scale` pixels per
	/// unit, and shows its subject in the middle of the room a drawer showing `shown` leaves, from 0 hidden to 1 in place,
	/// at `center` units from the left. Returns how wide the subject shows, in units, so the window can decide whether
	/// the drawer makes room for it.
	pub(crate) fn present(&mut self, index: usize, viewport: [f32; 2], scale: f32, center: f32) -> f32 {
		self.resize(index, viewport.map(|side| (side * scale).round() as u32));
		let framing = &mut self.slots[index].framing;
		let offset = center - viewport[0] * 0.5;
		if (framing.offset, framing.height) != (offset, viewport[1]) {
			framing.shift(offset, viewport[1]);
			self.place(index);
		}
		self.slots[index].framing.width(viewport[1])
	}

	/// Makes the slot's image `extent` pixels large, when it isn't already.
	pub(crate) fn resize(&mut self, index: usize, extent: [u32; 2]) {
		let slot = &mut self.slots[index];
		let extent = Extent::rectangle(extent[0].max(1), extent[1].max(1));
		if slot.extent != extent {
			slot.extent = extent;
			self.extents.send(SetRenderImageExtent::new(slot.render_image, extent));
		}
	}

	/// Moves the slot's camera and subject to where its framing puts them.
	fn place(&self, index: usize) {
		let slot = &self.slots[index];
		let transforms = self.scope.channel::<TransformationUpdate>();
		transforms.send(TransformationUpdate::new(slot.camera, slot.framing.camera_transform()));
		if let Some(subject) = slot.subject {
			transforms.send(TransformationUpdate::new(subject, slot.framing.subject_transform()));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::direction_from_orientation;

	/// Where the subject's point at `local`, in its own coordinates, stands in the world.
	fn placed(framing: &Framing, local: Vector) -> Point {
		let transform = framing.subject_transform();
		transform.get_position() + transform.get_orientation().rotate_vector(local)
	}

	#[test]
	fn the_camera_looks_at_the_subject_from_its_framing_distance() {
		let framing = Framing::new(
			Point::new(1.0, 2.0, 3.0),
			Vector::new(0.0, 0.0, 0.0),
			2.0,
			Vector::new(1.0, 1.0, 1.0),
		);
		let transform = framing.camera_transform();
		let toward = framing.target - transform.get_position();
		assert!(
			(toward.length() - 2.0 * FRAMING).abs() < 1e-4,
			"it stands {FRAMING} radii away"
		);
		let view = direction_from_orientation(transform.get_orientation()).into_vector();
		assert!((view - toward.normalized().unwrap().into_vector()).length() < 1e-4);
	}

	#[test]
	fn the_subject_turns_about_its_center_and_the_way_the_drag_moves() {
		let center = Vector::new(0.5, 1.0, -0.25);
		let mut framing = Framing::new(Point::new(10.0, 0.0, 0.0), center, 1.0, Vector::new(0.5, 0.5, 0.5));
		assert!(
			(placed(&framing, center) - framing.target).length() < 1e-5,
			"the center starts at the target"
		);

		// The point facing the camera, under the ball's center, moves right with a drag to the right.
		let [forward, right, _] = Framing::screen_axes();
		let facing = center - forward;
		let before = placed(&framing, facing);
		framing.turn([0.0, 0.0], [0.1, 0.0]);
		let moved = placed(&framing, facing) - before;
		assert!(moved.dot(right) > 0.05, "the facing point followed the drag: {moved:?}");
		assert!(
			(placed(&framing, center) - framing.target).length() < 1e-5,
			"the center stays at the target"
		);
	}

	#[test]
	fn a_drag_from_the_middle_of_the_ball_to_its_rim_turns_half_a_turn() {
		let mut framing = Framing::new(Point::origin(), Vector::new(0.0, 0.0, 0.0), 1.0, Vector::new(0.5, 0.5, 0.5));
		let [forward, ..] = Framing::screen_axes();
		framing.turn([0.0, 0.0], [1.0, 0.0]);
		// The ball's points are a quarter turn apart, and the arcball turns by twice that.
		let facing = placed(&framing, -forward);
		assert!(
			(facing - Point::origin() - forward).length() < 1e-4,
			"the facing side turned away: {facing:?}"
		);
	}

	#[test]
	fn a_shifted_camera_shows_the_subject_that_many_pixels_from_the_middle() {
		let mut framing = Framing::new(
			Point::new(3.0, 1.0, -2.0),
			Vector::new(0.0, 0.0, 0.0),
			1.0,
			Vector::new(0.5, 0.5, 0.5),
		);
		framing.shift(-150.0, 720.0);
		let [_, right, _] = Framing::screen_axes();
		let toward = framing.target - framing.camera_transform().get_position();
		let meters_per_pixel = 2.0 * framing.distance * Framing::half_view() / 720.0;
		assert!(
			(toward.dot(right) / meters_per_pixel + 150.0).abs() < 1e-2,
			"the subject shows 150 pixels left"
		);
	}

	#[test]
	fn the_subject_shows_wider_as_the_camera_comes_closer() {
		let mut framing = Framing::new(Point::origin(), Vector::new(0.0, 0.0, 0.0), 1.0, Vector::new(0.5, 0.5, 0.5));
		let framed = framing.width(720.0);
		assert!(
			framed > 0.0 && framed < 720.0,
			"framed, it fits the picture's height: {framed}"
		);
		framing.zoom(4.0);
		assert!(framing.width(720.0) > framed);
	}

	#[test]
	fn a_thin_subject_shows_as_wide_as_its_box_not_its_sphere() {
		// A flat panel, wide and tall but thin, held by a sphere as wide as it is tall.
		let half = Vector::new(1.0, 1.0, 0.05);
		let framing = Framing::new(Point::origin(), Vector::new(0.0, 0.0, 0.0), half.length(), half);
		assert!(
			framing.box_width(720.0) < framing.sphere_width(720.0),
			"the box is the tighter bound"
		);
		assert_eq!(framing.width(720.0), framing.box_width(720.0));
	}

	#[test]
	fn the_box_turns_with_the_subject() {
		// A rod shows as wide as its length across the screen, which a quarter turn about the screen's up changes.
		let half = Vector::new(1.0, 0.05, 0.05);
		let mut framing = Framing::new(Point::origin(), Vector::new(0.0, 0.0, 0.0), half.length(), half);
		let before = framing.width(720.0);
		framing.turn([0.0, 0.0], [0.7071, 0.0]);
		let turned = framing.width(720.0);
		assert!((turned - before).abs() > before * 0.1, "{before} before, {turned} turned");
		assert!(turned <= framing.sphere_width(720.0));
	}

	#[test]
	fn a_camera_inside_the_box_finds_the_subject_wider_than_any_picture() {
		let half = Vector::new(10.0, 0.1, 10.0);
		let framing = Framing::new(Point::origin(), Vector::new(0.0, 0.0, 0.0), 0.5, half);
		assert_eq!(framing.box_width(720.0), f32::INFINITY);
		assert!(framing.width(720.0).is_finite(), "the sphere still bounds it");
	}

	#[test]
	fn zooming_stays_between_the_nearest_and_farthest_the_camera_comes() {
		let mut framing = Framing::new(Point::origin(), Vector::new(0.0, 0.0, 0.0), 1.0, Vector::new(0.5, 0.5, 0.5));
		framing.zoom(100.0);
		assert!((framing.distance - ZOOM_RANGE[0]).abs() < 1e-5);
		framing.zoom(-100.0);
		assert!((framing.distance - ZOOM_RANGE[1]).abs() < 1e-5);
		// The camera sees its subject from as far as it goes.
		assert!(framing.camera().far() > framing.distance + framing.radius);
	}

	#[test]
	fn the_ball_holds_points_inside_it_and_its_rim_outside() {
		assert_eq!(ball_point([0.0, 0.0]), [0.0, 0.0, 1.0]);
		assert_eq!(ball_point([0.6, 0.0]), [0.6, 0.0, 0.8]);
		assert_eq!(ball_point([0.0, -2.0]), [0.0, -1.0, 0.0]);
	}
}
