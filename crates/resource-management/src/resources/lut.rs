use crate::{resource, solver::SolveErrors, Model, Reference, ReferenceModel, Resource, Solver};

/// The `LutKind` enum describes the lookup-table layout a LUT resource represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Lut {
	pub kind: LutKind,
	pub size: u32,
	pub domain_min: [f32; 3],
	pub domain_max: [f32; 3],
}

impl Resource for Lut {
	fn get_class(&self) -> &'static str {
		"Lut"
	}

	type Model = Lut;
}

impl Model for Lut {
	fn get_class() -> &'static str {
		"Lut"
	}
}

impl<'de> Solver<'de, Reference<Lut>> for ReferenceModel<Lut> {
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Lut>, SolveErrors> {
		let (resource, reader) = storage_backend.read(self.id()).ok_or_else(|| SolveErrors::StorageError)?;
		let lut: Lut = crate::from_slice(&resource.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, lut, reader))
	}
}
