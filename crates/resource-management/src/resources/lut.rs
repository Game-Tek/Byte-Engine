/// The `LutKind` enum identifies the sample layout of a lookup-table resource.
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum LutKind {
	OneDimensional,
	ThreeDimensional,
}

impl LutKind {
	pub fn expected_entry_count(self, size: u32) -> Option<usize> {
		match self {
			LutKind::OneDimensional => usize::try_from(size).ok(),
			LutKind::ThreeDimensional => usize::try_from(size.checked_pow(3)?).ok(),
		}
	}
}

/// The `Lut` struct carries the metadata needed to interpret baked LUT sample data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Lut {
	pub kind: LutKind,
	pub size: u32,
	pub domain_min: [f32; 3],
	pub domain_max: [f32; 3],
}

/// The `LutParameters` struct exists so every reader of a LUT, on the GPU or in a bake, maps colors to its texels the
/// same way. It mirrors `LutParameters` in the engine's `layouts/lut.besl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LutParameters {
	pub domain_min: [f32; 4],
	pub domain_scale: [f32; 4],
	/// The scale and offset that take a normalized color to texel centers, then the look strength.
	pub sampling: [f32; 4],
}

impl Lut {
	/// Returns the parameters a shader that takes the `Lut` layer reads this table with, at look strength `strength`.
	///
	/// Next, write them to the buffer bound as `lut_parameters`.
	pub fn parameters(&self, strength: f32) -> LutParameters {
		debug_assert!(
			self.size > 0,
			"LUT size is zero. The most likely cause is accepting an empty LUT resource."
		);
		debug_assert!(
			(0..3).all(|index| {
				self.domain_min[index].is_finite()
					&& self.domain_max[index].is_finite()
					&& self.domain_max[index] > self.domain_min[index]
			}),
			"LUT domain is invalid. The most likely cause is a non-finite or non-increasing color range."
		);
		let domain_scale: [f32; 3] = std::array::from_fn(|index| 1.0 / (self.domain_max[index] - self.domain_min[index]));
		let size = self.size as f32;
		LutParameters {
			domain_min: [self.domain_min[0], self.domain_min[1], self.domain_min[2], 0.0],
			domain_scale: [domain_scale[0], domain_scale[1], domain_scale[2], 0.0],
			sampling: [(size - 1.0) / size, 0.5 / size, strength, 0.0],
		}
	}
}

super::impl_direct_resource!(Lut, "Lut");
