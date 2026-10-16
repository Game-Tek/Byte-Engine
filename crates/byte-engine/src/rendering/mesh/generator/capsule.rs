use std::{
	borrow::Cow,
	hash::{Hash as _, Hasher},
};

use maths_rs::{Vec3f, Vec4f, cross};

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

/// The `CapsuleMeshGenerator` struct provides unbranded mesh-space streams for an upright capsule centred on the
/// origin, shaped like [`crate::physics::Shapes::capsule`] so that a collider can be drawn where it is.
#[derive(Clone)]
pub struct CapsuleMeshGenerator {
	radius: f32,
	height: f32,
	material: Option<&'static str>,
}

/// Rows of vertices from each pole to the equator, both included.
const CAP_RINGS: u32 = 4;
/// Columns of quads around the axis.
const SEGMENTS: u32 = 12;

impl CapsuleMeshGenerator {
	/// Creates a capsule `height` tall, caps included, around the vertical axis. A height below twice the radius
	/// draws a sphere.
	pub fn new(radius: f32, height: f32) -> Self {
		Self {
			radius,
			height,
			material: None,
		}
	}

	/// Draws the capsule with the material resource `material` instead of the default generated-mesh material.
	pub fn with_material(mut self, material: &'static str) -> Self {
		self.material = Some(material);
		self
	}

	/// Returns the polar angle and vertical offset of every row, top pole first. The two equator rows sit at the ends
	/// of the cylinder, so the quads between them form its side.
	fn rows(&self) -> impl Iterator<Item = (f32, f32)> {
		let half_cylinder = (self.height * 0.5 - self.radius).max(0.0);
		(0..=CAP_RINGS)
			.map(move |ring| (ring as f32 / CAP_RINGS as f32, half_cylinder))
			.chain((0..=CAP_RINGS).map(move |ring| (1.0 + ring as f32 / CAP_RINGS as f32, -half_cylinder)))
			.map(|(quarters, offset)| (quarters * std::f32::consts::FRAC_PI_2, offset))
	}

	fn row_count() -> u32 {
		2 * (CAP_RINGS + 1)
	}

	fn columns() -> impl Iterator<Item = f32> + Clone {
		(0..=SEGMENTS).map(|segment| 2.0 * std::f32::consts::PI * segment as f32 / SEGMENTS as f32)
	}

	fn normal(theta: f32, phi: f32) -> Vec3f {
		Vec3f::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
	}

	fn tangent(phi: f32) -> Vec3f {
		Vec3f::new(-phi.sin(), 0.0, phi.cos())
	}
}

impl MeshGenerator for CapsuleMeshGenerator {
	fn positions(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Owned(
			self.rows()
				.flat_map(|(theta, offset)| {
					Self::columns().map(move |phi| {
						let normal = Self::normal(theta, phi) * self.radius;
						(normal.x, normal.y + offset, normal.z)
					})
				})
				.collect(),
		)
	}

	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Owned(
			self.rows()
				.flat_map(|(theta, _)| {
					Self::columns().map(move |phi| {
						let normal = Self::normal(theta, phi);
						(normal.x, normal.y, normal.z)
					})
				})
				.collect(),
		)
	}

	fn tangents(&self) -> Cow<'_, [Vec3f]> {
		Cow::Owned(self.rows().flat_map(|_| Self::columns().map(Self::tangent)).collect())
	}

	fn bitangents(&self) -> Cow<'_, [Vec3f]> {
		Cow::Owned(
			self.rows()
				.flat_map(|(theta, _)| Self::columns().map(move |phi| cross(Self::normal(theta, phi), Self::tangent(phi))))
				.collect(),
		)
	}

	fn uvs(&self) -> Cow<'_, [(f32, f32)]> {
		let last_row = (Self::row_count() - 1) as f32;
		Cow::Owned(
			(0..Self::row_count())
				.flat_map(|row| (0..=SEGMENTS).map(move |segment| (segment as f32 / SEGMENTS as f32, row as f32 / last_row)))
				.collect(),
		)
	}

	fn indices(&self) -> Cow<'_, [u32]> {
		let mut indices = Vec::with_capacity(((Self::row_count() - 1) * SEGMENTS * 6) as usize);
		for row in 0..Self::row_count() - 1 {
			for segment in 0..SEGMENTS {
				let top_left = row * (SEGMENTS + 1) + segment;
				let bottom_left = (row + 1) * (SEGMENTS + 1) + segment;
				indices.extend_from_slice(&[
					top_left,
					top_left + 1,
					bottom_left,
					bottom_left,
					top_left + 1,
					bottom_left + 1,
				]);
			}
		}
		Cow::Owned(indices)
	}

	fn colors(&self) -> Option<Cow<'_, [Vec4f]>> {
		None
	}

	fn meshlet_indices(&self) -> Option<Cow<'_, [u8]>> {
		const {
			assert!(
				2 * (CAP_RINGS + 1) * (SEGMENTS + 1) <= u8::MAX as u32 + 1,
				"Capsule meshlet index exceeds u8. The most likely cause is increasing capsule tessellation without widening meshlet indices."
			)
		};
		Some(Cow::Owned(self.indices().iter().map(|index| *index as u8).collect()))
	}

	fn material(&self) -> Option<&str> {
		self.material
	}

	fn hash(&self) -> u64 {
		let mut hasher = std::hash::DefaultHasher::new();
		"capsule".hash(&mut hasher);
		self.radius.to_bits().hash(&mut hasher);
		self.height.to_bits().hash(&mut hasher);
		self.material.hash(&mut hasher);
		hasher.finish()
	}

	fn clone_box(&self) -> Box<dyn MeshGenerator> {
		Box::new(self.clone())
	}
}

impl From<CapsuleMeshGenerator> for Box<dyn MeshGenerator> {
	fn from(value: CapsuleMeshGenerator) -> Self {
		Box::new(value)
	}
}

impl From<CapsuleMeshGenerator> for MeshSource {
	fn from(value: CapsuleMeshGenerator) -> Self {
		MeshSource::Generated(Box::new(value))
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::{Vec3f, cross, dot, length};

	use super::CapsuleMeshGenerator;
	use crate::rendering::mesh::generator::{MeshGenerator, validate_triangle_indices};

	#[test]
	fn surface_spans_the_height_and_every_frame_is_right_handed() {
		let capsule = CapsuleMeshGenerator::new(0.3, 1.8);
		let positions = capsule.positions();
		let heights = positions.iter().map(|position| position.1);
		assert!((heights.clone().fold(f32::MIN, f32::max) - 0.9).abs() < 1e-5);
		assert!((heights.fold(f32::MAX, f32::min) + 0.9).abs() < 1e-5);

		for (((position, normal), tangent), bitangent) in positions
			.iter()
			.zip(capsule.normals().iter())
			.zip(capsule.tangents().iter())
			.zip(capsule.bitangents().iter())
		{
			let normal = Vec3f::new(normal.0, normal.1, normal.2);
			// Every point lies one radius from the axis segment, along its normal.
			let axis = Vec3f::new(0.0, position.1.clamp(-0.6, 0.6), 0.0);
			assert!(length(Vec3f::new(position.0, position.1, position.2) - axis - normal * 0.3) < 1e-5);
			assert!(dot(normal, *tangent).abs() < 1e-4);
			assert!(dot(cross(*tangent, *bitangent), normal) > 0.9999);
		}

		assert_eq!(validate_triangle_indices(&capsule.indices(), positions.len()), Ok(()));
		assert_eq!(capsule.meshlet_indices().unwrap().len(), capsule.indices().len());
	}
}
