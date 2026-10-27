//! Scenes, ray queries, and ray tracing pipelines for host-side evaluation of ray tracing shaders.
//!
//! The VM traces rays the way the GHI does on the GPU, without the hardware's acceleration: it tests every triangle of
//! every instance, in order. Results match the GPU except where the BESL reference leaves them open, such as which of
//! two equally near triangles a ray reports.

use std::sync::Arc;

use super::*;

/// The `BottomLevelStructure` struct holds the triangle geometries that VM ray queries test, in the object space of the
/// instances that place it.
///
/// It mirrors a GHI bottom-level acceleration structure: geometry `i` is the `i`th list of triangles, and a hit on it
/// reports `i` as its geometry index. Place it in a scene with an [`Instance`], then build the scene with
/// [`AccelerationStructure::new`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BottomLevelStructure {
	geometries: Vec<Vec<[[f32; 3]; 3]>>,
}

impl BottomLevelStructure {
	/// Creates a structure from its geometries. Each geometry lists its triangles by their three vertex positions.
	pub fn new(geometries: Vec<Vec<[[f32; 3]; 3]>>) -> Self {
		Self { geometries }
	}
}

/// The `Instance` struct places a [`BottomLevelStructure`] in a VM scene, the way a GHI ray tracing instance does.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
	/// The rows of the affine transform that maps the structure's object space to world space.
	pub transform: [[f32; 4]; 3],
	/// The value that hits on this instance report as their `custom_index`.
	pub custom_index: u32,
	/// The instance takes part in ray queries whose mask shares a set bit with this one.
	pub mask: u8,
	/// The value a hit adds to its geometry index to give its record.
	pub hit_group_offset: u32,
	/// The structure the instance places.
	pub bottom_level: Arc<BottomLevelStructure>,
}

/// The `AccelerationStructure` struct is the scene that VM ray queries test rays against, mirroring a GHI top-level
/// acceleration structure built from its instances.
///
/// Bind it to an `AccelerationStructure` descriptor with [`DescriptorBindings::bind_acceleration_structure`], then run
/// the shader with [`ExecutableProgram::run_main`] or [`RayTracingPipeline::trace_rays`].
pub struct AccelerationStructure {
	instances: Vec<PlacedInstance>,
}

/// The `PlacedInstance` struct keeps an instance with the inverse of its transform, which maps world-space rays into the
/// instance's object space. `None` marks a transform that flattens the structure, which rays never hit.
struct PlacedInstance {
	instance: Instance,
	world_to_object: Option<[[f32; 4]; 3]>,
}

impl AccelerationStructure {
	/// Builds a scene from its instances. Rays test the instances in this order, which decides ties.
	pub fn new(instances: Vec<Instance>) -> Self {
		Self {
			instances: instances
				.into_iter()
				.map(|instance| PlacedInstance {
					world_to_object: invert_affine(&instance.transform),
					instance,
				})
				.collect(),
		}
	}

	/// Returns the nearest hit along `ray`, or a miss. Of equally near triangles, the first in instance, geometry, and
	/// triangle order wins, so results are deterministic.
	pub(super) fn intersect(&self, ray: &Ray) -> RayHit {
		self.hits(ray)
			.reduce(|nearest, hit| if hit.distance < nearest.distance { hit } else { nearest })
			.unwrap_or_else(|| RayHit::miss(ray))
	}

	/// Reports whether any triangle lies along `ray`, stopping at the first one found.
	pub(super) fn occludes(&self, ray: &Ray) -> bool {
		self.hits(ray).next().is_some()
	}

	/// Visits every triangle hit within the ray's interval, in instance, geometry, and triangle order.
	///
	/// Rays are moved into each instance's object space instead of moving its triangles into world space. An affine
	/// map keeps the ray parameter and the barycentrics of every point, so the results are those of a world-space
	/// test, as on the GPU.
	fn hits<'a>(&'a self, ray: &'a Ray) -> impl Iterator<Item = RayHit> + 'a {
		self.instances
			.iter()
			.filter(move |placed| u32::from(placed.instance.mask) & ray.mask != 0)
			.filter_map(|placed| Some((&placed.instance, placed.world_to_object.as_ref()?)))
			.flat_map(move |(instance, world_to_object)| {
				let origin = transform_point(world_to_object, ray.origin);
				let direction = transform_vector(world_to_object, ray.direction);
				instance
					.bottom_level
					.geometries
					.iter()
					.enumerate()
					.flat_map(move |(geometry_index, triangles)| {
						triangles.iter().enumerate().filter_map(move |(primitive_index, triangle)| {
							let (distance, barycentrics) = intersect_triangle(origin, direction, triangle)?;
							(distance >= ray.t_min && distance <= ray.t_max).then(|| RayHit {
								record: instance.hit_group_offset.wrapping_add(geometry_index as u32),
								distance,
								custom_index: instance.custom_index,
								geometry_index: geometry_index as u32,
								primitive_index: primitive_index as u32,
								barycentrics,
								origin: ray.origin,
								direction: ray.direction,
							})
						})
					})
			})
	}
}

/// The `Ray` struct holds the operands of one ray query after their interval was checked.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ray {
	pub(super) mask: u32,
	pub(super) origin: [f32; 3],
	pub(super) direction: [f32; 3],
	pub(super) t_min: f32,
	pub(super) t_max: f32,
}

impl Ray {
	/// Creates a ray query, rejecting an interval whose result is undefined on the GPU: a negative or NaN `t_min`, or a
	/// `t_max` below `t_min`.
	pub(super) fn new(mask: u32, origin: [f32; 3], direction: [f32; 3], t_min: f32, t_max: f32) -> Result<Self, VmError> {
		if !(t_min >= 0.0 && t_max >= t_min) {
			return Err(VmError::InvalidRayInterval {
				t_min: format!("{t_min:?}"),
				t_max: format!("{t_max:?}"),
			});
		}
		Ok(Self {
			mask,
			origin,
			direction,
			t_min,
			t_max,
		})
	}
}

/// The `RayHit` struct holds the members of a BESL `Hit`, for the ray queries that produce one and the `invoke` calls
/// and closest-hit inputs that read one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RayHit {
	pub(super) record: u32,
	pub(super) distance: f32,
	pub(super) custom_index: u32,
	pub(super) geometry_index: u32,
	pub(super) primitive_index: u32,
	pub(super) barycentrics: [f32; 2],
	pub(super) origin: [f32; 3],
	pub(super) direction: [f32; 3],
}

impl RayHit {
	/// Returns the miss of `ray`: a distance of `-1.0`, the ray's origin and direction, and zero in every other member.
	fn miss(ray: &Ray) -> Self {
		Self {
			record: 0,
			distance: -1.0,
			custom_index: 0,
			geometry_index: 0,
			primitive_index: 0,
			barycentrics: [0.0; 2],
			origin: ray.origin,
			direction: ray.direction,
		}
	}

	/// Reports whether `invoke` treats this hit as a miss: its distance is negative or NaN.
	pub(super) fn is_miss(&self) -> bool {
		self.distance < 0.0 || self.distance.is_nan()
	}

	/// Builds the BESL `Hit` value of `hit_type`. The members follow the order of the built-in `Hit` struct.
	pub(super) fn to_value(self, hit_type: &ValueType) -> Value {
		Value::Struct {
			value_type: hit_type.clone(),
			fields: Arc::from([
				Value::U32(self.record),
				Value::F32(self.distance),
				Value::U32(self.custom_index),
				Value::U32(self.geometry_index),
				Value::U32(self.primitive_index),
				Value::Vec2F(self.barycentrics),
				Value::Vec3F(self.origin),
				Value::Vec3F(self.direction),
			]),
		}
	}

	/// Reads a BESL `Hit` value, whose members follow the order of the built-in `Hit` struct.
	pub(super) fn from_value(value: &Value) -> Result<Self, VmError> {
		let mismatch = || VmError::TypeMismatch {
			expected: crate::HIT_STRUCT.to_string(),
			found: value.value_type().name().to_string(),
		};
		let Value::Struct { fields, .. } = value else {
			return Err(mismatch());
		};
		match **fields {
			[
				Value::U32(record),
				Value::F32(distance),
				Value::U32(custom_index),
				Value::U32(geometry_index),
				Value::U32(primitive_index),
				Value::Vec2F(barycentrics),
				Value::Vec3F(origin),
				Value::Vec3F(direction),
			] => Ok(Self {
				record,
				distance,
				custom_index,
				geometry_index,
				primitive_index,
				barycentrics,
				origin,
				direction,
			}),
			_ => Err(mismatch()),
		}
	}
}

/// The `RayTracingPipeline` struct pairs a ray generation program with the closest-hit programs of its hit groups, so
/// host code can run ray tracing shaders the way a GHI ray tracing pipeline runs them.
///
/// Build one with [`RayTracingPipeline::new`], bind the resources its shaders read, and call
/// [`RayTracingPipeline::trace_rays`].
pub struct RayTracingPipeline {
	pub(super) ray_generation: ExecutableProgram,
	pub(super) hit_groups: Vec<ExecutableProgram>,
}

impl RayTracingPipeline {
	/// Creates a pipeline from a ray generation program and the closest-hit program of each hit group, in hit group
	/// order.
	///
	/// It rejects what the GPU wouldn't catch: a hit group that isn't a closest hit, a closest hit that calls `invoke`
	/// or a ray query, and a closest hit whose payload differs from the payload the ray generation program passes to
	/// `invoke`.
	pub fn new(ray_generation: ExecutableProgram, hit_groups: Vec<ExecutableProgram>) -> Result<Self, VmError> {
		if ray_generation.closest_hit_payload.is_some() {
			return Err(VmError::NotARayGenerationShader);
		}
		for (hit_group, program) in hit_groups.iter().enumerate() {
			let Some(payload) = &program.closest_hit_payload else {
				return Err(VmError::NotAClosestHitShader { hit_group });
			};
			let traces_rays = program
				.functions
				.iter()
				.flat_map(|function| &function.instructions)
				.any(|instruction| matches!(instruction, Instruction::RayTracing(_)));
			if traces_rays {
				return Err(VmError::RayTracingInClosestHit { hit_group });
			}
			if let Some(expected) = &ray_generation.invoke_payload
				&& expected != payload
			{
				return Err(VmError::PayloadMismatch {
					hit_group,
					expected: expected.name().to_string(),
					found: payload.name().to_string(),
				});
			}
		}
		Ok(Self {
			ray_generation,
			hit_groups,
		})
	}

	/// Runs the ray generation program once for each point of `launch_size`, with `records` as the shader binding
	/// table: record `i` runs hit group `records[i]`.
	///
	/// A height or depth of 0 counts as 1, and a width of 0 runs nothing, as on the GPU. Points run one after another,
	/// with x varying fastest, so writes that collide resolve the same way every time.
	pub fn trace_rays(
		&self,
		descriptors: &mut DescriptorBindings<'_>,
		records: &[u32],
		launch_size: [u32; 3],
	) -> Result<(), VmError> {
		if let Some((record, &hit_group)) = records
			.iter()
			.enumerate()
			.find(|(_, hit_group)| **hit_group as usize >= self.hit_groups.len())
		{
			return Err(VmError::HitGroupOutOfRange {
				record,
				hit_group,
				count: self.hit_groups.len(),
			});
		}
		let [width, height, depth] = launch_size;
		let launch_size = [width, height.max(1), depth.max(1)];
		if width == 0 {
			return Ok(());
		}
		let config = ExecutionConfig::default();
		for z in 0..launch_size[2] {
			for y in 0..launch_size[1] {
				for x in 0..launch_size[0] {
					let trace = TraceContext {
						hit_groups: &self.hit_groups,
						records,
						launch_index: [x, y, z],
						launch_size,
					};
					self.ray_generation.run_ray_generation(descriptors, &config, trace)?;
				}
			}
		}
		Ok(())
	}
}

/// The `TraceContext` struct gives one ray generation invocation its launch point and the hit groups that its `invoke`
/// calls run.
#[derive(Clone, Copy)]
pub(super) struct TraceContext<'a> {
	pub(super) hit_groups: &'a [ExecutableProgram],
	pub(super) records: &'a [u32],
	pub(super) launch_index: [u32; 3],
	pub(super) launch_size: [u32; 3],
}

/// Intersects a ray with one triangle, from either side, and returns the ray parameter of the hit and the weights of the
/// triangle's second and third vertices. This is the Möller–Trumbore test. Each check is written so that a NaN fails
/// it, so a NaN ray hits nothing.
fn intersect_triangle(origin: [f32; 3], direction: [f32; 3], [v0, v1, v2]: &[[f32; 3]; 3]) -> Option<(f32, [f32; 2])> {
	let edge1 = subtract(*v1, *v0);
	let edge2 = subtract(*v2, *v0);
	let p = cross_product(direction, edge2);
	let determinant = dot_product(edge1, p);
	// A zero determinant means the ray runs parallel to the triangle's plane, or the triangle has no area.
	if determinant == 0.0 {
		return None;
	}
	let inverse = 1.0 / determinant;
	let s = subtract(origin, *v0);
	let u = dot_product(s, p) * inverse;
	if !(0.0..=1.0).contains(&u) {
		return None;
	}
	let q = cross_product(s, edge1);
	let v = dot_product(direction, q) * inverse;
	if !(v >= 0.0 && u + v <= 1.0) {
		return None;
	}
	Some((dot_product(edge2, q) * inverse, [u, v]))
}

/// Inverts the affine transform whose rows are `transform`, or returns `None` when its linear part is singular.
fn invert_affine(transform: &[[f32; 4]; 3]) -> Option<[[f32; 4]; 3]> {
	let [r0, r1, r2] = transform.map(|row| [row[0], row[1], row[2]]);
	// Column j of the inverse of a 3x3 matrix is the cross product of the two rows other than j, over the determinant.
	let c0 = cross_product(r1, r2);
	let c1 = cross_product(r2, r0);
	let c2 = cross_product(r0, r1);
	let determinant = dot_product(r0, c0);
	if determinant == 0.0 || !determinant.is_finite() {
		return None;
	}
	let inverse = 1.0 / determinant;
	let linear = [
		[c0[0] * inverse, c1[0] * inverse, c2[0] * inverse],
		[c0[1] * inverse, c1[1] * inverse, c2[1] * inverse],
		[c0[2] * inverse, c1[2] * inverse, c2[2] * inverse],
	];
	let translation = [transform[0][3], transform[1][3], transform[2][3]];
	Some(linear.map(|row| [row[0], row[1], row[2], -dot_product(row, translation)]))
}

/// Applies an affine transform, given by its rows, to a point.
fn transform_point(transform: &[[f32; 4]; 3], point: [f32; 3]) -> [f32; 3] {
	transform.map(|row| dot_product([row[0], row[1], row[2]], point) + row[3])
}

/// Applies the linear part of an affine transform, given by its rows, to a direction.
fn transform_vector(transform: &[[f32; 4]; 3], vector: [f32; 3]) -> [f32; 3] {
	transform.map(|row| dot_product([row[0], row[1], row[2]], vector))
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|index| left[index] - right[index])
}
