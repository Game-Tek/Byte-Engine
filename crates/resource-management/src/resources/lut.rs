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

super::impl_direct_resource!(Lut, "Lut");
