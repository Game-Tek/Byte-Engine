//! Deterministic CPU texture and sampler fixtures for VM resource operations.

use super::*;

/// The `Texture` struct provides deterministic CPU texels for shader sampling, image access, and atomic assertions.
#[derive(Debug)]
pub struct Texture {
	pub(super) width: u32,
	pub(super) height: u32,
	depth: u32,
	texels: Vec<Texel>,
	mips: Vec<Texture>,
}

#[derive(Clone, Copy, Debug)]
enum Texel {
	Zero,
	Float([f32; 4]),
	U32(u32),
}

impl Texel {
	const fn kind(self) -> &'static str {
		match self {
			Self::Zero => "untyped zero",
			Self::Float(_) => "float RGBA",
			Self::U32(_) => "u32",
		}
	}

	fn float(self) -> Result<[f32; 4], VmError> {
		match self {
			Self::Zero => Ok([0.0; 4]),
			Self::Float(value) => Ok(value),
			value => Err(VmError::TextureFormatMismatch {
				expected: "float RGBA",
				found: value.kind(),
			}),
		}
	}

	fn u32(self) -> Result<u32, VmError> {
		match self {
			Self::Zero => Ok(0),
			Self::U32(value) => Ok(value),
			value => Err(VmError::TextureFormatMismatch {
				expected: "u32",
				found: value.kind(),
			}),
		}
	}
}

/// The `SamplerReductionMode` enum selects how the VM combines a linear sampler's neighboring texels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SamplerReductionMode {
	/// Blends texels using their bilinear weights.
	#[default]
	WeightedAverage,
	/// Selects the component-wise minimum across the sampler footprint.
	Min,
	/// Selects the component-wise maximum across the sampler footprint.
	Max,
}

/// The `Sampler` struct supplies deterministic sampler state for combined texture bindings in VM tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sampler {
	reduction_mode: SamplerReductionMode,
}

impl Sampler {
	/// Creates a linear clamp sampler with the requested reduction mode.
	pub const fn new(reduction_mode: SamplerReductionMode) -> Self {
		Self { reduction_mode }
	}
}

fn reduce_rgba(samples: [[f32; 4]; 4], reduce: fn(f32, f32) -> f32) -> [f32; 4] {
	std::array::from_fn(|channel| {
		reduce(
			reduce(samples[0][channel], samples[1][channel]),
			reduce(samples[2][channel], samples[3][channel]),
		)
	})
}

impl Texture {
	pub fn new(width: u32, height: u32) -> Result<Self, VmError> {
		Self::new_3d(width, height, 1)
	}

	/// Creates a CPU texture with three-dimensional addressing for VM texture tests.
	pub fn new_3d(width: u32, height: u32, depth: u32) -> Result<Self, VmError> {
		if width == 0 || height == 0 || depth == 0 {
			return Err(VmError::InvalidTextureDimensions { width, height, depth });
		}

		let texel_count = (width as usize)
			.checked_mul(height as usize)
			.and_then(|area| area.checked_mul(depth as usize))
			.ok_or(VmError::TextureTexelCountOverflow { width, height, depth })?;
		texel_count
			.checked_mul(std::mem::size_of::<Texel>())
			.filter(|byte_count| *byte_count <= isize::MAX as usize)
			.ok_or(VmError::TextureTexelCountOverflow { width, height, depth })?;

		// Fallible reservation keeps hostile or accidental dimensions on the VM error path.
		let mut texels = Vec::new();
		texels
			.try_reserve_exact(texel_count)
			.map_err(|_| VmError::TextureTexelCountOverflow { width, height, depth })?;
		texels.resize(texel_count, Texel::Zero);
		Ok(Self {
			width,
			height,
			depth,
			texels,
			mips: Vec::new(),
		})
	}

	/// Adds the next explicit mip level used by CPU shader fixtures.
	pub fn add_mip(&mut self, mip: Texture) {
		self.mips.push(mip);
	}

	pub fn write(&mut self, coord: [u32; 2], value: [f32; 4]) -> Result<(), VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		self.texels[index] = Texel::Float(value);
		Ok(())
	}

	/// Writes one texel in a three-dimensional CPU texture.
	pub fn write_3d(&mut self, coord: [u32; 3], value: [f32; 4]) -> Result<(), VmError> {
		let index = self.texel_index(coord)?;
		self.texels[index] = Texel::Float(value);
		Ok(())
	}

	/// Writes one unsigned integer texel for integer image and atomic tests.
	pub fn write_u32(&mut self, coord: [u32; 2], value: u32) -> Result<(), VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		self.texels[index] = Texel::U32(value);
		Ok(())
	}

	/// Fetches one texel without interpolation.
	pub fn fetch(&self, coord: [u32; 2]) -> Result<Value, VmError> {
		Ok(Value::Vec4F(self.fetch_texel([coord[0], coord[1], 0])?))
	}

	/// Fetches one texel from a two-dimensional array layer without interpolation.
	pub fn fetch_array(&self, coord: [u32; 2], layer: u32) -> Result<Value, VmError> {
		Ok(Value::Vec4F(self.fetch_texel([coord[0], coord[1], layer])?))
	}

	/// Fetches one unsigned integer texel without interpolation.
	pub fn fetch_u32(&self, coord: [u32; 2]) -> Result<Value, VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		Ok(Value::U32(self.texels[index].u32()?))
	}

	/// Samples one texel using bilinear interpolation in normalized UV space.
	pub fn sample(&self, uv: [f32; 2]) -> Result<Value, VmError> {
		self.sample_with_sampler(uv, Sampler::default())
	}

	/// Samples one texel using the sampler state attached to a combined texture binding.
	pub(super) fn sample_with_sampler(&self, uv: [f32; 2], sampler: Sampler) -> Result<Value, VmError> {
		let (x0, x1, tx) = normalized_linear_axis(uv[0], self.width);
		let (y0, y1, ty) = normalized_linear_axis(uv[1], self.height);
		let samples = [
			self.fetch_texel([x0, y0, 0])?,
			self.fetch_texel([x1, y0, 0])?,
			self.fetch_texel([x0, y1, 0])?,
			self.fetch_texel([x1, y1, 0])?,
		];
		let sampled = match sampler.reduction_mode {
			SamplerReductionMode::WeightedAverage => {
				let top = lerp_rgba(samples[0], samples[1], tx);
				let bottom = lerp_rgba(samples[2], samples[3], tx);
				lerp_rgba(top, bottom, ty)
			}
			SamplerReductionMode::Min => reduce_rgba(samples, f32::min),
			SamplerReductionMode::Max => reduce_rgba(samples, f32::max),
		};

		Ok(Value::Vec4F(sampled))
	}

	/// Samples an explicit LOD, clamping to the coarsest mip like GPU texture sampling.
	pub fn sample_lod(&self, uv: [f32; 2], lod: f32) -> Result<Value, VmError> {
		self.sample_lod_with_sampler(uv, lod, Sampler::default())
	}

	/// Samples an explicit LOD using the sampler state attached to a combined texture binding.
	pub(super) fn sample_lod_with_sampler(&self, uv: [f32; 2], lod: f32, sampler: Sampler) -> Result<Value, VmError> {
		let level = if lod.is_finite() { lod.max(0.0) as usize } else { 0 };
		if level == 0 {
			return self.sample_with_sampler(uv, sampler);
		}
		let texture = self.mips.get(level - 1).unwrap_or_else(|| self.mips.last().unwrap_or(self));
		texture.sample_with_sampler(uv, sampler)
	}

	/// Samples one array layer at an explicit LOD with deterministic reduction behavior.
	pub(super) fn sample_array_lod_with_sampler(
		&self,
		uv: [f32; 2],
		layer: u32,
		lod: f32,
		sampler: Sampler,
	) -> Result<Value, VmError> {
		let level = if lod.is_finite() { lod.max(0.0) as usize } else { 0 };
		let texture = if level == 0 {
			self
		} else {
			self.mips.get(level - 1).unwrap_or_else(|| self.mips.last().unwrap_or(self))
		};
		let (x0, x1, tx) = normalized_linear_axis(uv[0], texture.width);
		let (y0, y1, ty) = normalized_linear_axis(uv[1], texture.height);
		let samples = [
			texture.fetch_texel([x0, y0, layer])?,
			texture.fetch_texel([x1, y0, layer])?,
			texture.fetch_texel([x0, y1, layer])?,
			texture.fetch_texel([x1, y1, layer])?,
		];
		let sampled = match sampler.reduction_mode {
			SamplerReductionMode::WeightedAverage => {
				let top = lerp_rgba(samples[0], samples[1], tx);
				let bottom = lerp_rgba(samples[2], samples[3], tx);
				lerp_rgba(top, bottom, ty)
			}
			SamplerReductionMode::Min => reduce_rgba(samples, f32::min),
			SamplerReductionMode::Max => reduce_rgba(samples, f32::max),
		};
		Ok(Value::Vec4F(sampled))
	}

	/// Samples a three-dimensional texture using trilinear interpolation.
	pub fn sample_3d(&self, uvw: [f32; 3]) -> Result<Value, VmError> {
		let x = normalized_linear_axis(uvw[0], self.width);
		let y = normalized_linear_axis(uvw[1], self.height);
		let z = normalized_linear_axis(uvw[2], self.depth);
		let low = [x.0, y.0, z.0];
		let high = [x.1, y.1, z.1];
		let factor = [x.2, y.2, z.2];
		let low_plane = lerp_rgba(
			lerp_rgba(
				self.fetch_texel([low[0], low[1], low[2]])?,
				self.fetch_texel([high[0], low[1], low[2]])?,
				factor[0],
			),
			lerp_rgba(
				self.fetch_texel([low[0], high[1], low[2]])?,
				self.fetch_texel([high[0], high[1], low[2]])?,
				factor[0],
			),
			factor[1],
		);
		let high_plane = lerp_rgba(
			lerp_rgba(
				self.fetch_texel([low[0], low[1], high[2]])?,
				self.fetch_texel([high[0], low[1], high[2]])?,
				factor[0],
			),
			lerp_rgba(
				self.fetch_texel([low[0], high[1], high[2]])?,
				self.fetch_texel([high[0], high[1], high[2]])?,
				factor[0],
			),
			factor[1],
		);
		Ok(Value::Vec4F(lerp_rgba(low_plane, high_plane, factor[2])))
	}

	fn fetch_texel(&self, coord: [u32; 3]) -> Result<[f32; 4], VmError> {
		let index = self.texel_index(coord)?;
		self.texels[index].float()
	}

	fn texel_index(&self, coord: [u32; 3]) -> Result<usize, VmError> {
		let [x, y, z] = coord;
		if x >= self.width || y >= self.height || z >= self.depth {
			return Err(VmError::TextureAccessOutOfBounds {
				x,
				y,
				z,
				width: self.width,
				height: self.height,
				depth: self.depth,
			});
		}

		Ok(((z as usize) * self.height as usize + y as usize) * self.width as usize + x as usize)
	}

	pub(super) fn contains_2d(&self, coord: [u32; 2]) -> bool {
		coord[0] < self.width && coord[1] < self.height
	}

	pub(super) fn atomic_or(&mut self, coord: [u32; 2], value: u32) -> Result<u32, VmError> {
		let index = self.texel_index([coord[0], coord[1], 0])?;
		let previous = self.texels[index].u32()?;
		let updated = previous | value;
		self.texels[index] = Texel::U32(updated);
		Ok(previous)
	}
}
