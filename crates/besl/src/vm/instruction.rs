//! Private instruction and operator types shared by lowering and execution.

use super::{ResourceSlot, Value, ValueType};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Instruction {
	LoadLiteral {
		register: usize,
		value: Value,
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
	ExtractDynamic {
		register: usize,
		source: usize,
		index: usize,
		count: usize,
		value_type: ValueType,
	},
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
	JumpIfZero {
		register: usize,
		target: usize,
	},
	Jump {
		target: usize,
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
	ThreadIdx {
		register: usize,
	},
	ThreadPosition {
		register: usize,
	},
	ThreadId {
		register: usize,
	},
	ThreadgroupPosition {
		register: usize,
	},
	SubgroupBallot {
		register: usize,
		predicate: usize,
	},
	SubgroupBallotAny {
		register: usize,
		mask: usize,
	},
	SubgroupBallotFindLsb {
		register: usize,
		mask: usize,
	},
	SubgroupBallotCount {
		register: usize,
		mask: usize,
	},
	SubgroupBallotAndNot {
		register: usize,
		mask: usize,
		removed: usize,
	},
	SubgroupBroadcastU32 {
		register: usize,
		value: usize,
		source_lane: usize,
	},
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
	AtomicAddWorkgroup {
		register: usize,
		name: String,
		index: Option<usize>,
		count: usize,
		value: usize,
	},
	AtomicCompareExchangeWorkgroup {
		register: usize,
		name: String,
		index: Option<usize>,
		count: usize,
		expected: usize,
		desired: usize,
	},
	WorkgroupBarrier,
	SetTaskMeshOutputCount {
		count: usize,
	},
	SetMeshOutputCounts {
		vertex_count: usize,
		primitive_count: usize,
	},
	SetMeshVertexPosition {
		index: usize,
		position: usize,
	},
	SetMeshTriangle {
		index: usize,
		triangle: usize,
	},
	SetMeshPrimitiveRenderTargetArrayIndex {
		index: usize,
		array_index: usize,
	},
	LoadLocal {
		register: usize,
		local: usize,
	},
	StoreLocal {
		local: usize,
		register: usize,
	},
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
		count: usize,
		index: usize,
		value_type: ValueType,
	},
	FetchTexture {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
	},
	FetchTextureU32 {
		register: usize,
		slot: ResourceSlot,
		coord: usize,
	},
	SampleTexture {
		register: usize,
		slot: ResourceSlot,
		uv: usize,
		lod: Option<usize>,
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
		count: usize,
		index: usize,
		value_type: ValueType,
		register: usize,
	},
	AtomicAddBuffer {
		register: usize,
		slot: ResourceSlot,
		offset: usize,
		stride: usize,
		count: usize,
		index: Option<usize>,
		value: usize,
	},
	AtomicCompareExchangeBuffer {
		register: usize,
		slot: ResourceSlot,
		offset: usize,
		stride: usize,
		count: usize,
		index: Option<usize>,
		expected: usize,
		desired: usize,
	},
	Call {
		register: Option<usize>,
		function: usize,
		arguments: Vec<usize>,
	},
	Return {
		register: Option<usize>,
	},
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
