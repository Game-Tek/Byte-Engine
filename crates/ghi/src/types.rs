#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// Enumerates the available layouts.
pub enum Layouts {
	/// The layout is undefined. We don't mind what the layout is.
	Undefined,
	/// The image will be used as render target.
	RenderTarget,
	/// The resource will be used in a transfer operation.
	Transfer,
	/// The resource will be used as a presentation source.
	Present,
	/// The resource will be used as a read only sample source.
	Read,
	/// The resource will be used as a read/write storage.
	General,
	/// The resource will be used as a shader binding table.
	ShaderBindingTable,
	/// Indirect.
	Indirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Enumerates the available filtering modes, primarily used in samplers.
pub enum FilteringModes {
	/// Closest mode filtering. Rounds floating point coordinates to the nearest pixel.
	Closest,
	/// Linear mode filtering. Blends samples linearly across neighbouring pixels.
	Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Enumerates the available sampling reduction modes.
/// The sampling reduction mode is used to determine how to reduce/combine the samples of neighbouring texels when sampling an image.
pub enum SamplingReductionModes {
	/// The average of the samples. Weighted by the proximity of the sample to the sample point.
	WeightedAverage,
	/// The minimum of the samples is taken.
	Min,
	/// The maximum of the samples is taken.
	Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Enumerates the available sampler addressing modes.
pub enum SamplerAddressingModes {
	/// Repeat mode addressing.
	Repeat,
	/// Mirror mode addressing.
	Mirror,
	/// Clamp mode addressing.
	Clamp,
	/// Border mode addressing.
	Border {},
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UseCases {
	STATIC,
	DYNAMIC,
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	/// Bit flags for the available resource uses.
	pub struct Uses : u32 {
		/// Resource will be used as a vertex buffer.
		const Vertex = 1 << 0;
		/// Resource will be used as an index buffer.
		const Index = 1 << 1;
		/// Resource will be used as a uniform buffer.
		const Uniform = 1 << 2;
		/// Resource will be used as a storage buffer.
		const Storage = 1 << 3;
		/// Resource will be used as an indirect buffer.
		const Indirect = 1 << 4;
		/// Resource will be used as an image.
		const Image = 1 << 5;
		/// Resource will be used as a render target.
		const RenderTarget = 1 << 6;
		/// Resource will be used as an input attachment.
		const InputAttachment = 1 << 15;
		/// Resource will be used as a depth stencil.
		const DepthStencil = 1 << 7;
		/// Resource will be used as an acceleration structure.
		const AccelerationStructure = 1 << 8;
		/// Resource will be used as a transfer source.
		const TransferSource = 1 << 9;
		/// Resource will be used as a transfer destination.
		const TransferDestination = 1 << 10;
		/// Resource will be used as a shader binding table.
		const ShaderBindingTable = 1 << 11;
		/// Resource will be used as a acceleration structure build scratch buffer.
		const AccelerationStructureBuildScratch = 1 << 12;

		const AccelerationStructureBuild = 1 << 13;

		const Clear = 1 << 14;

		/// Resource will be used as a source for a blit operation.
		const BlitSource = 1 << 9;
		/// Resource will be used as a destination for a blit operation.
		const BlitDestination = 1 << 10;
	}
}
