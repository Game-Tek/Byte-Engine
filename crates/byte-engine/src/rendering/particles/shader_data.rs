//! CPU mirrors of the particle shaders' data. Field order and sizes match `ParticleFrame` in
//! `assets/rendering/particles/prepare.besl` and in the kernels resource management generates from `.particles`
//! assets.

use ghi::pod::{Mat4f, Mat4x3f};

/// How many emitters of one system can be alive, or have particles in flight, at once.
pub(crate) const MAX_EMITTERS: usize = 256;

pub(crate) const FRAME_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(0);
pub(crate) const PARTICLES_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);
pub(crate) const DRAWS_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(2);
pub(crate) const DISPATCH_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(3);

/// The `ShaderSpawn` struct names the emitter that owns a run of this frame's new particles.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShaderSpawn {
	/// The index of the run's first particle among this frame's new particles.
	pub(crate) first: u32,
	/// The emitter slot the run spawns from.
	pub(crate) emitter: u32,
}

/// The `ParticleFrameData` struct is everything one system's kernels need from the CPU for one frame.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ParticleFrameData {
	pub(crate) delta_time: f32,
	/// Varies every frame so new particles get new random values.
	pub(crate) seed: u32,
	/// How many particles the emitters ask for this frame. The GPU spawns fewer when the buffer is nearly full.
	pub(crate) spawn_total: u32,
	/// How many entries of `spawns` are used.
	pub(crate) spawn_count: u32,
	/// The buffer half and draw record this frame writes. The previous frame wrote the other one.
	pub(crate) side: u32,
	/// `1` when the other half holds nothing usable, such as on the first frame or after an idle stretch.
	pub(crate) reset: u32,
	/// How many particles fit in one half of the particle buffer.
	pub(crate) capacity: u32,
	pub(crate) padding: u32,
	pub(crate) spawns: [ShaderSpawn; MAX_EMITTERS],
	/// Each emitter slot's transform.
	pub(crate) emitters: [Mat4x3f; MAX_EMITTERS],
}

/// The `ShaderParticle` struct sizes one particle in the GPU-only particle buffer; the CPU never reads it.
///
/// The kernels spell every component as a scalar, because backends pad a `vec3f` struct member to 16 bytes.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShaderParticle {
	pub(crate) position: [f32; 3],
	pub(crate) life: f32,
	pub(crate) velocity: [f32; 3],
	pub(crate) packed: u32,
}

/// The `ParticlePushConstants` struct places one camera's particle draw.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ParticlePushConstants {
	pub(crate) view_projection: Mat4f,
	/// The camera position in `xyz` and the camera exposure in `w`. One vector keeps the layout free of `vec3f` padding.
	pub(crate) camera: [f32; 4],
}

const _: () = assert!(std::mem::offset_of!(ParticleFrameData, spawns) == 32);
const _: () = assert!(std::mem::offset_of!(ParticleFrameData, emitters) == 32 + 8 * MAX_EMITTERS);
const _: () = assert!(size_of::<ShaderParticle>() == 32);
const _: () = assert!(size_of::<ParticlePushConstants>() == 80);
