/// The `DebugSceneManager` struct owns reusable geometry and the retained debug scene shared by render sinks.
///
/// Applications normally create it through
/// [`crate::application::graphics::setup_debug_mesh_render_pass`] and manage
/// meshes through the returned [`crate::core::factory::Factory`].
pub struct DebugSceneManager {
	frame: Option<ghi::FrameKey>,
	debug_meshes: DebugMeshStore,
	meshes: DebugMeshes,
}

impl DebugSceneManager {
	/// Creates the shared scene and uploads each reusable unit mesh once.
	pub fn new(
		context: &mut ghi::implementation::Context,
		listener: DefaultListener<CreateMessage<DebugMesh>>,
		delete_listener: DefaultListener<DeleteMessage>,
	) -> Self {
		Self {
			frame: None,
			debug_meshes: DebugMeshStore::new(listener, delete_listener),
			meshes: DebugMeshes::new(context),
		}
	}

	/// Applies pending lifecycle messages once and reuses the retained scene for every later sink in that frame.
	pub(crate) fn debug_meshes(&mut self, frame: ghi::FrameKey) -> impl Iterator<Item = DebugMesh> + '_ {
		if self.frame != Some(frame) {
			self.debug_meshes.update();
			self.frame = Some(frame);
		}

		self.debug_meshes.values()
	}

	/// Returns the reusable GPU mesh selected by one expanded shape instance.
	pub(crate) fn mesh(&self, kind: MeshKind) -> ghi::MeshHandle {
		match kind {
			MeshKind::Sphere => self.meshes.sphere,
			MeshKind::Box => self.meshes.r#box,
			MeshKind::Cylinder => self.meshes.cylinder,
		}
	}
}

/// The `DebugMeshStore` struct retains validated debug meshes by their world handle.
struct DebugMeshStore {
	listener: DefaultListener<CreateMessage<DebugMesh>>,
	delete_listener: DefaultListener<DeleteMessage>,
	meshes: Vec<(Handle, DebugMesh)>,
}

impl DebugMeshStore {
	/// Creates an empty retained store connected to future lifecycle messages.
	fn new(listener: DefaultListener<CreateMessage<DebugMesh>>, delete_listener: DefaultListener<DeleteMessage>) -> Self {
		Self {
			listener,
			delete_listener,
			meshes: Vec::new(),
		}
	}

	/// Applies valid creations and replacements before terminal deletions.
	fn update(&mut self) {
		while let Some(message) = self.listener.read() {
			let handle = message.handle();
			let debug_mesh = message.into_data();
			if valid_debug_mesh(debug_mesh) {
				if let Some((_, retained)) = self.meshes.iter_mut().find(|(retained_handle, _)| *retained_handle == handle) {
					*retained = debug_mesh;
				} else {
					self.meshes.push((handle, debug_mesh));
				}
			} else {
				log::warn!(
					"Debug mesh was not created or updated. The most likely cause is a non-finite color or transform, a non-positive extent, or a zero-length segment."
				);
			}
		}
		while let Some(message) = self.delete_listener.read() {
			if let Some(index) = self.meshes.iter().position(|(handle, _)| handle == message.handle()) {
				self.meshes.remove(index);
			}
		}
	}

	/// Visits every retained debug mesh without exposing storage identity.
	fn values(&self) -> impl Iterator<Item = DebugMesh> + '_ {
		self.meshes.iter().map(|(_, debug_mesh)| *debug_mesh)
	}
}

/// The `DebugMeshes` struct owns the canonical GPU meshes reused by every debug shape.
struct DebugMeshes {
	sphere: ghi::MeshHandle,
	r#box: ghi::MeshHandle,
	cylinder: ghi::MeshHandle,
}

impl DebugMeshes {
	/// Uploads the canonical unit sphere, box, and +Z cylinder used by shape transforms.
	fn new(context: &mut ghi::implementation::Context) -> Self {
		Self {
			sphere: upload_generated_mesh(context, &SphereMeshGenerator::new()),
			r#box: upload_generated_mesh(context, &BoxMeshGenerator::new()),
			cylinder: upload_cylinder(context),
		}
	}
}

/// The `MeshKind` enum identifies one canonical mesh owned by the debug scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeshKind {
	Sphere,
	Box,
	Cylinder,
}

/// Converts one existing mesh generator into the position-only u16 layout used by debug pipelines.
fn upload_generated_mesh(
	context: &mut ghi::implementation::Context,
	generator: &dyn crate::rendering::mesh::generator::MeshGenerator,
) -> ghi::MeshHandle {
	let positions = generator.positions().iter().map(|&(x, y, z)| [x, y, z]).collect::<Vec<_>>();
	let indices = generator
		.indices()
		.iter()
		.map(|&index| {
			u16::try_from(index).expect(
				"Debug mesh index is too large. The most likely cause is that a unit debug mesh exceeded the GHI u16 mesh-index contract.",
			)
		})
		.collect::<Vec<_>>();

	upload_mesh(context, &positions, &indices)
}

/// Builds a closed unit cylinder extending from -1 to +1 on the local Z axis.
fn upload_cylinder(context: &mut ghi::implementation::Context) -> ghi::MeshHandle {
	const SEGMENTS: usize = 12;
	let mut positions = Vec::with_capacity(SEGMENTS * 2 + 2);
	for segment in 0..SEGMENTS {
		let angle = std::f32::consts::TAU * segment as f32 / SEGMENTS as f32;
		let (sin, cos) = angle.sin_cos();
		positions.push([cos, sin, -1.0]);
		positions.push([cos, sin, 1.0]);
	}
	let bottom_center = positions.len() as u16;
	positions.push([0.0, 0.0, -1.0]);
	let top_center = positions.len() as u16;
	positions.push([0.0, 0.0, 1.0]);

	let mut indices = Vec::with_capacity(SEGMENTS * 12);
	for segment in 0..SEGMENTS {
		let next = (segment + 1) % SEGMENTS;
		let bottom = (segment * 2) as u16;
		let top = bottom + 1;
		let next_bottom = (next * 2) as u16;
		let next_top = next_bottom + 1;
		indices.extend_from_slice(&[
			bottom,
			next_bottom,
			top,
			top,
			next_bottom,
			next_top,
			bottom_center,
			next_bottom,
			bottom,
			top_center,
			top,
			next_top,
		]);
	}

	upload_mesh(context, &positions, &indices)
}

/// Uploads tightly packed position and u16 index data as one retained GHI mesh.
fn upload_mesh(context: &mut ghi::implementation::Context, positions: &[[f32; 3]], indices: &[u16]) -> ghi::MeshHandle {
	context.add_mesh_from_vertices_and_indices(
		positions.len() as u32,
		indices.len() as u32,
		bytemuck::cast_slice(positions),
		bytemuck::cast_slice(indices),
		&[ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)],
	)
}

/// Rejects malformed messages before they can put non-finite transforms into push constants.
pub(crate) fn valid_debug_mesh(debug_mesh: DebugMesh) -> bool {
	let color = debug_mesh.color();
	if ![color.r, color.g, color.b, color.a].into_iter().all(f32::is_finite) {
		return false;
	}

	match debug_mesh.shape() {
		DebugShape::Sphere { center, radius } => math::is_finite(center) && radius.is_finite() && radius > 0.0,
		DebugShape::Box {
			center, half_extents, ..
		} => {
			math::is_finite(center)
				&& [half_extents.x(), half_extents.y(), half_extents.z()]
					.into_iter()
					.all(|extent| extent.is_finite() && extent > 0.0)
		}
		DebugShape::Capsule { start, end, radius } => valid_endpoints(start, end, true) && radius.is_finite() && radius > 0.0,
		DebugShape::Segment { start, end } => valid_endpoints(start, end, false),
	}
}

/// Checks endpoints and rejects non-zero axes whose subtraction or measured length overflows.
fn valid_endpoints(start: math::Point, end: math::Point, allow_zero_length: bool) -> bool {
	if !math::is_finite(start) || !math::is_finite(end) {
		return false;
	}
	if start == end {
		return allow_zero_length;
	}

	(end - start)
		.normalize_with_length()
		.is_ok_and(|(_, length)| length.is_finite())
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Point, Vector};
	use utils::RGBA;

	use super::*;
	use crate::{
		core::{channel::Channel as _, factory::Factory},
		rendering::debug::DebugMesh,
	};

	/// Verifies supported geometry passes the message boundary while malformed geometry and color do not.
	#[test]
	fn validation_accepts_supported_shapes_and_rejects_invalid_geometry() {
		let valid = [
			DebugShape::Sphere {
				center: Point::origin(),
				radius: 1.0,
			},
			DebugShape::Box {
				center: Point::origin(),
				half_extents: Vector::new(1.0, 2.0, 3.0),
				orientation: Orientation::identity(),
			},
			DebugShape::Capsule {
				start: Point::origin(),
				end: Point::new(0.0, 1.0, 0.0),
				radius: 0.5,
			},
			DebugShape::Segment {
				start: Point::origin(),
				end: Point::new(1.0, 0.0, 0.0),
			},
		];
		assert!(
			valid
				.into_iter()
				.all(|shape| valid_debug_mesh(DebugMesh::new(shape, RGBA::white())))
		);

		let invalid = [
			DebugShape::Sphere {
				center: Point::origin(),
				radius: 0.0,
			},
			DebugShape::Box {
				center: Point::origin(),
				half_extents: Vector::new(1.0, -1.0, 1.0),
				orientation: Orientation::identity(),
			},
			DebugShape::Capsule {
				start: Point::origin(),
				end: Point::origin(),
				radius: f32::NAN,
			},
			DebugShape::Segment {
				start: Point::origin(),
				end: Point::origin(),
			},
		];
		assert!(
			invalid
				.into_iter()
				.all(|shape| !valid_debug_mesh(DebugMesh::new(shape, RGBA::white())))
		);
		assert!(!valid_debug_mesh(DebugMesh::new(
			DebugShape::Sphere {
				center: Point::origin(),
				radius: 1.0,
			},
			RGBA::new(f32::NAN, 1.0, 1.0, 1.0),
		)));
		assert!(!valid_debug_mesh(DebugMesh::new(
			DebugShape::Segment {
				start: Point::new(f32::MAX, 0.0, 0.0),
				end: Point::new(-f32::MAX, 0.0, 0.0),
			},
			RGBA::white(),
		)));
	}

	/// Verifies creations persist across updates, replacements preserve identity, and deletion retires the mesh.
	#[test]
	fn retained_meshes_persist_until_their_handle_is_deleted() {
		let factory = Factory::new();
		let deletions = crate::core::channel::DefaultChannel::new();
		let mut meshes = DebugMeshStore::new(factory.listener(), deletions.listener());
		let initial = DebugMesh::new(
			DebugShape::Sphere {
				center: Point::origin(),
				radius: 1.0,
			},
			RGBA::white(),
		);

		let handle = factory.create(initial);
		meshes.update();
		assert_eq!(meshes.values().collect::<Vec<_>>(), [initial]);
		meshes.update();
		assert_eq!(meshes.values().collect::<Vec<_>>(), [initial]);

		let replacement = DebugMesh::new(
			DebugShape::Segment {
				start: Point::origin(),
				end: Point::new(1.0, 0.0, 0.0),
			},
			RGBA::new(1.0, 0.0, 0.0, 1.0),
		);
		factory.derive(handle, replacement);
		meshes.update();
		assert_eq!(meshes.values().collect::<Vec<_>>(), [replacement]);

		deletions.send(DeleteMessage::new(handle));
		meshes.update();
		assert_eq!(meshes.values().count(), 0);
	}

	/// Verifies a malformed replacement cannot discard the last valid retained representation.
	#[test]
	fn invalid_replacement_preserves_the_retained_mesh() {
		let factory = Factory::new();
		let deletions = crate::core::channel::DefaultChannel::new();
		let mut meshes = DebugMeshStore::new(factory.listener(), deletions.listener());
		let initial = DebugMesh::new(
			DebugShape::Sphere {
				center: Point::origin(),
				radius: 1.0,
			},
			RGBA::white(),
		);
		let handle = factory.create(initial);
		meshes.update();

		factory.derive(
			handle,
			DebugMesh::new(
				DebugShape::Sphere {
					center: Point::origin(),
					radius: f32::NAN,
				},
				RGBA::white(),
			),
		);
		meshes.update();

		assert_eq!(meshes.values().collect::<Vec<_>>(), [initial]);
	}
}

use ghi::context::ContextCreate as _;

use crate::{
	core::{
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
	},
	rendering::{
		debug::{DebugMesh, DebugShape},
		mesh::generator::{BoxMeshGenerator, MeshGenerator as _, SphereMeshGenerator},
	},
};
