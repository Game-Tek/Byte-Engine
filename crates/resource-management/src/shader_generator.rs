use utils::Extent;

/// Generates a graphics API consumable shader from a BESL shader program definition.
pub trait ShaderGenerator {

}

pub enum Stages {
	Vertex,
	Compute {
		local_size: Extent,
	},
	Task,
	Mesh {
		maximum_vertices: u32,
		maximum_primitives: u32,
		local_size: Extent,
	},
	Fragment,
}

pub enum MatrixLayouts {
	RowMajor,
	ColumnMajor,
}

pub struct GLSLSettings {
	pub(crate) version: String,
}

impl Default for GLSLSettings {
	fn default() -> Self {
		Self {
			version: "450".to_string(),
		}
	}
}

pub struct ShaderGenerationSettings {
	pub(crate) glsl: GLSLSettings,
	pub(crate) stage: Stages,
	pub(crate) matrix_layout: MatrixLayouts,
	pub(crate) name: String,
}

impl ShaderGenerationSettings {
	pub fn compute(extent: Extent) -> ShaderGenerationSettings {
		Self::from_stage(Stages::Compute { local_size: extent })
	}

	pub fn task() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Task)
	}

	pub fn mesh(maximum_vertices: u32, maximum_primitives: u32, local_size: Extent) -> ShaderGenerationSettings {
		Self::from_stage(Stages::Mesh{ maximum_vertices, maximum_primitives, local_size })
	}

	pub fn fragment() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Fragment)
	}

	pub fn vertex() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Vertex)
	}

	fn from_stage(stage: Stages) -> Self {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage, matrix_layout: MatrixLayouts::RowMajor, name: "shader".to_string() }
	}

	pub fn name(mut self, name: String) -> Self {
		self.name = name;
		self
	}
}
