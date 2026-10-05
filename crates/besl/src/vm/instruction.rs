//! Private instruction and operator types shared by lowering and execution.

use super::{ResourceSlot, SamplerReductionMode, Value, ValueType};

/// The `Instruction` enum is one VM operation. Lowering pushes it and execution dispatches it by group, so each
/// group handler can match its own sub-enum exhaustively.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Instruction {
	Value(ValueInstruction),
	Numeric(NumericInstruction),
	Local(LocalInstruction),
	Workgroup(WorkgroupInstruction),
	MeshOutput(MeshOutputInstruction),
	Buffer(BufferInstruction),
	Texture(TextureInstruction),
	Image(ImageInstruction),
	Control(ControlInstruction),
	/// Builds a ballot mask from every subgroup lane's `predicate`. Under workgroup scheduling it suspends the lane
	/// until the whole subgroup arrives.
	SubgroupBallot {
		register: usize,
		predicate: usize,
	},
	/// Copies `value` from the subgroup lane selected by `source_lane`. `value_type` is the scalar type the
	/// broadcast accepts, `u32` or `f32`.
	SubgroupBroadcast {
		register: usize,
		value: usize,
		source_lane: usize,
		value_type: ValueType,
	},
	WorkgroupBarrier,
}

/// Lets lowering emit any instruction group directly, through `impl Into<Instruction>`.
macro_rules! instruction_groups {
	($($group:ident => $variant:ident),+ $(,)?) => {
		$(impl From<$group> for Instruction {
			fn from(instruction: $group) -> Self {
				Self::$variant(instruction)
			}
		})+
	};
}

instruction_groups!(
	ValueInstruction => Value,
	NumericInstruction => Numeric,
	LocalInstruction => Local,
	WorkgroupInstruction => Workgroup,
	MeshOutputInstruction => MeshOutput,
	BufferInstruction => Buffer,
	TextureInstruction => Texture,
	ImageInstruction => Image,
	ControlInstruction => Control,
);

/// The `ValueInstruction` enum groups the instructions that load literals and resource handles, or build and take
/// apart aggregate register values.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ValueInstruction {
	LoadLiteral {
		register: usize,
		value: Value,
	},
	LoadResourceIndexed {
		register: usize,
		slot: ResourceSlot,
		index: usize,
		count: usize,
		value_type: ValueType,
	},
	Construct {
		register: usize,
		value_type: ValueType,
		components: Vec<usize>,
	},
	Extract {
		register: usize,
		source: usize,
		index: usize,
		value_type: ValueType,
	},
	/// Copies `source` with the member at `index` replaced by `value`, the inverse of `Extract`.
	Insert {
		register: usize,
		source: usize,
		index: usize,
		value: usize,
	},
	ExtractDynamic {
		register: usize,
		source: usize,
		index: usize,
		count: usize,
		value_type: ValueType,
	},
	/// Copies `source` with the element selected by the `index` register replaced by `value`, the inverse of
	/// `ExtractDynamic`.
	InsertDynamic {
		register: usize,
		source: usize,
		index: usize,
		count: usize,
		value: usize,
	},
}

/// The `NumericInstruction` enum groups the pure arithmetic, comparison, and math-library instructions. Each reads its
/// operand registers and writes one result register.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum NumericInstruction {
	Arithmetic {
		register: usize,
		operator: ArithmeticOperator,
		left: usize,
		right: usize,
	},
	Compare {
		register: usize,
		operator: ComparisonOperator,
		left: usize,
		right: usize,
	},
	DotProduct {
		register: usize,
		left: usize,
		right: usize,
	},
	CrossProduct {
		register: usize,
		left: usize,
		right: usize,
	},
	Length {
		register: usize,
		value: usize,
	},
	Normalize {
		register: usize,
		value: usize,
	},
	Reflect {
		register: usize,
		incident: usize,
		normal: usize,
	},
	UnaryScalar {
		register: usize,
		operator: ScalarUnaryOperator,
		value: usize,
	},
	FloatPredicate {
		register: usize,
		predicate: FloatPredicate,
		value: usize,
	},
	RoundToVec2I {
		register: usize,
		value: usize,
	},
	BinaryScalar {
		register: usize,
		operator: ScalarBinaryOperator,
		left: usize,
		right: usize,
	},
	TernaryScalar {
		register: usize,
		operator: ScalarTernaryOperator,
		first: usize,
		second: usize,
		third: usize,
	},
}

/// The `LocalInstruction` enum groups the frame-local storage, invocation builtin, and non-suspending subgroup mask
/// instructions.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum LocalInstruction {
	LoadLocal {
		register: usize,
		local: usize,
	},
	StoreLocal {
		local: usize,
		register: usize,
	},
	/// Writes one invocation builtin, such as the thread index, into `register`.
	LoadBuiltin {
		register: usize,
		builtin: InvocationBuiltin,
	},
	/// Reduces the ballot `mask` to one scalar with `operator`.
	SubgroupMask {
		register: usize,
		operator: SubgroupMaskOperator,
		mask: usize,
	},
	SubgroupBallotAndNot {
		register: usize,
		mask: usize,
		removed: usize,
	},
}

/// The `WorkgroupInstruction` enum groups the task-payload and workgroup-shared storage instructions.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum WorkgroupInstruction {
	LoadTaskPayload {
		register: usize,
		name: String,
		index: usize,
		count: usize,
		value_type: ValueType,
	},
	StoreTaskPayload {
		name: String,
		index: usize,
		count: usize,
		value_type: ValueType,
		value: usize,
	},
	LoadWorkgroup {
		register: usize,
		name: String,
		index: Option<usize>,
		count: usize,
		value_type: ValueType,
	},
	StoreWorkgroup {
		name: String,
		index: Option<usize>,
		count: usize,
		value_type: ValueType,
		value: usize,
	},
	AtomicWorkgroup {
		register: usize,
		operation: AtomicOperation,
		name: String,
		index: Option<usize>,
		count: usize,
		value_type: ValueType,
		value: usize,
	},
	AtomicCompareExchangeWorkgroup {
		register: usize,
		name: String,
		index: Option<usize>,
		count: usize,
		value_type: ValueType,
		expected: usize,
		desired: usize,
	},
	SetTaskMeshOutputCount {
		count: usize,
	},
}

/// The `MeshOutputInstruction` enum groups the instructions that write the bound mesh-shader output capture. Each
/// variant names the output it sets.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MeshOutputInstruction {
	Counts { vertex_count: usize, primitive_count: usize },
	VertexPosition { index: usize, position: usize },
	Triangle { index: usize, triangle: usize },
	PrimitiveRenderTargetArrayIndex { index: usize, array_index: usize },
}

/// The `BufferInstruction` enum groups the reads, writes, and atomics against bound buffers and push constants.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum BufferInstruction {
	LoadBuffer {
		register: usize,
		slot: ResourceSlot,
		offset: usize,
		value_type: ValueType,
	},
	LoadBufferIndexed {
		register: usize,
		slot: ResourceSlot,
		offset: usize,
		stride: usize,
		count: Option<usize>,
		index: usize,
		value_type: ValueType,
	},
	StoreBuffer {
		slot: ResourceSlot,
		offset: usize,
		value_type: ValueType,
		register: usize,
	},
	StoreBufferIndexed {
		slot: ResourceSlot,
		offset: usize,
		stride: usize,
		count: Option<usize>,
		index: usize,
		value_type: ValueType,
		register: usize,
	},
	AtomicBuffer {
		register: usize,
		operation: AtomicOperation,
		slot: ResourceSlot,
		offset: usize,
		stride: usize,
		count: Option<usize>,
		index: Option<usize>,
		value_type: ValueType,
		value: usize,
	},
	AtomicCompareExchangeBuffer {
		register: usize,
		slot: ResourceSlot,
		offset: usize,
		stride: usize,
		count: Option<usize>,
		index: Option<usize>,
		value_type: ValueType,
		expected: usize,
		desired: usize,
	},
}

/// The `TextureInstruction` enum groups the fetch, sample, and size queries against bound textures.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum TextureInstruction {
	FetchTexture {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
	},
	FetchTextureArray {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
		layer: usize,
	},
	FetchTextureU32 {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
	},
	/// Reads the first channel of the four texels around a normalized UV in mip zero, in GPU gather order.
	GatherTexture {
		register: usize,
		slot: ResourceSlot,
		uv: usize,
		layer: Option<usize>,
	},
	SampleTexture {
		register: usize,
		slot: ResourceSlot,
		uv: usize,
		layer: Option<usize>,
		lod: Option<usize>,
		reduction_mode: Option<SamplerReductionMode>,
	},
	SampleTexture3D {
		register: usize,
		slot: ResourceSlot,
		uvw: usize,
	},
	TextureSize {
		register: usize,
		slot: ResourceSlot,
	},
}

/// The `ImageInstruction` enum groups the reads, writes, atomics, and bounds guards against bound storage images.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ImageInstruction {
	ImageSize {
		register: usize,
		slot: ResourceSlot,
	},
	LoadImage {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
	},
	LoadImageU32 {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
	},
	GuardImageBounds {
		slot: ResourceSlot,
		coord: usize,
	},
	ImageAtomicOr {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
		value: usize,
	},
	WriteImage {
		slot: ResourceSlot,
		coord: usize,
		value: usize,
	},
}

/// The `ControlInstruction` enum groups the instructions that move the frame's instruction pointer, call functions, or
/// end the frame.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ControlInstruction {
	JumpIfZero {
		register: usize,
		target: usize,
	},
	Jump {
		target: usize,
	},
	/// Jumps to the target of the case whose label equals the scalar in `register`, or to `default`.
	/// Labels are the value's 32-bit pattern, sorted for binary search. Lowers a BESL `match`.
	Switch {
		register: usize,
		cases: Box<[(u32, usize)]>,
		default: usize,
	},
	Discard,
	Call {
		register: Option<usize>,
		function: usize,
		arguments: Vec<usize>,
	},
	Return {
		register: Option<usize>,
	},
}

/// The `InvocationBuiltin` enum names the per-invocation coordinates a shader can read from its
/// [`ExecutionConfig`](super::ExecutionConfig).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InvocationBuiltin {
	ThreadIdx,
	ThreadPosition,
	ThreadId,
	ThreadgroupPosition,
	SubgroupLaneIndex,
}

/// The `SubgroupMaskOperator` enum names the scalar reductions of a subgroup ballot mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SubgroupMaskOperator {
	/// Whether any lane bit is set.
	Any,
	/// The index of the lowest set lane bit, or `u32::MAX` when no bit is set.
	FindLsb,
	/// The number of set lane bits.
	Count,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArithmeticOperator {
	Add,
	Subtract,
	Multiply,
	Divide,
	Modulo,
	ShiftLeft,
	ShiftRight,
	BitwiseAnd,
	BitwiseOr,
	BitwiseXor,
	LogicalAnd,
	LogicalOr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComparisonOperator {
	Equal,
	NotEqual,
	LessThan,
	GreaterThan,
	LessThanOrEqual,
	GreaterThanOrEqual,
}

/// The `AtomicOperation` enum identifies one relaxed read-modify-write operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AtomicOperation {
	Exchange,
	Add,
	Subtract,
	Min,
	Max,
	And,
	Or,
	Xor,
}

/// The `FloatPredicate` enum identifies one IEEE floating-point classification query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FloatPredicate {
	Nan,
	Infinite,
	Finite,
	Normal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScalarUnaryOperator {
	Abs,
	Sqrt,
	Exp,
	Sin,
	Cos,
	Tan,
	Asin,
	Floor,
	Round,
	Fract,
	Radians,
	InverseSqrt,
	Log2,
	Fwidth,
	/// The index of the lowest set bit of a `u32`, or `u32::MAX` when no bit is set.
	FindLsb,
	FromF16ToF32,
	FromU32ToF32,
	FromI32ToF32,
	FromF32ToF16,
	FromU32ToF16,
	FromI32ToF16,
	FromF32ToU32,
	FromF16ToU32,
	FromU8ToU32,
	FromU16ToU32,
	FromU32ToU16,
	FromI32ToU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScalarBinaryOperator {
	Min,
	Max,
	Pow,
	Step,
	Atan2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScalarTernaryOperator {
	Smoothstep,
	Mix,
	Clamp,
	Fma,
}
