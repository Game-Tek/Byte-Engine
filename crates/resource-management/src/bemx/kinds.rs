//! The node kinds a `.bemx` file can use, as data: each kind's inputs, with their defaults, and its other fields.
//!
//! The reader in [`super::json`] accepts exactly the fields this table lists for a kind, and tools that edit graphs,
//! such as an editor's node canvas, draw a node's sockets and settings from it. The
//! [BEMX reference](/docs/reference/bemx) specifies what each kind computes.

use super::{COMBINE_INPUTS, NUMBERED_INPUTS, SWITCH_INPUTS};

/// The `InputDefault` enum is what an input of a node reads when a file leaves it out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputDefault {
	/// The node can't do without the input.
	Required,
	/// The input stays unconnected, and the node falls back to its own behavior, such as the shaded point's texture
	/// coordinates for an `image`'s `texcoord`.
	Unset,
	/// The input reads this constant: one number, or two to four.
	Value(&'static [f32]),
}

/// The `Input` struct is one input of a node kind: a value the node reads, which a file connects to another node or
/// writes as a constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
	/// The input's field name, as MaterialX names it, such as `in1`.
	pub name: &'static str,
	pub default: InputDefault,
}

/// The `SettingKind` enum names what a setting of a node holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingKind {
	/// A name, such as a material parameter's or a texture's.
	Text,
	/// A value type, such as `vector3` or `color3`.
	Type,
	/// A whole number from 0, such as a texture coordinate set or a channel.
	Index,
	/// Channel letters, such as `xyz` or `bgr`.
	Channels,
	/// A constant written in place: a number, or an array of two to four numbers.
	Value,
}

/// The `Setting` struct is one field of a node kind that isn't an input: it holds a value of its own and never
/// connects to another node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
	pub name: &'static str,
	pub kind: SettingKind,
	/// Whether a file must write it.
	pub required: bool,
}

/// The `NodeKind` struct is one kind of BEMX node, as a `.bemx` file spells it in a node's `node` field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeKind {
	/// The kind's name, the MaterialX category it follows, such as `multiply`.
	pub name: &'static str,
	pub inputs: &'static [Input],
	pub settings: &'static [Setting],
	/// Whether the node produces a value, which other nodes can read. Only the surface doesn't.
	pub produces: bool,
}

impl NodeKind {
	/// Whether a node of this kind may write `field`: its `name` and `node`, one of its inputs, or one of its settings.
	pub fn accepts(&self, field: &str) -> bool {
		matches!(field, "name" | "node")
			|| self.inputs.iter().any(|input| input.name == field)
			|| self.settings.iter().any(|setting| setting.name == field)
	}
}

/// Returns the node kind named `name`, such as `multiply`, or `None` when BEMX has no such kind.
pub fn kind(name: &str) -> Option<&'static NodeKind> {
	KINDS.iter().find(|kind| kind.name == name)
}

/// Every node kind BEMX has, grouped as the reference lists them.
pub const KINDS: &[NodeKind] = &[
	// Values and geometry.
	node("constant", &[], &[required("value", SettingKind::Value)]),
	node(
		"parameter",
		&[],
		&[required("parameter", SettingKind::Text), required("type", SettingKind::Type)],
	),
	node("position", &[], &[]),
	node("normal", &[], &[]),
	node("tangent", &[], &[]),
	node("bitangent", &[], &[]),
	node("viewdirection", &[], &[]),
	node("texcoord", &[], &[optional("index", SettingKind::Index)]),
	node("time", &[], &[]),
	node("screenposition", &[], &[]),
	node(
		"geompropvalue",
		&[],
		&[required("geomprop", SettingKind::Text), required("type", SettingKind::Type)],
	),
	node("scenecolor", &[unset("texcoord")], &[]),
	node("sceneposition", &[unset("texcoord")], &[]),
	node(
		"image",
		&[unset("texcoord")],
		&[required("texture", SettingKind::Text), optional("type", SettingKind::Type)],
	),
	// One input.
	unary("absval"),
	unary("floor"),
	unary("ceil"),
	unary("round"),
	unary("sign"),
	unary("sqrt"),
	unary("exp"),
	unary("ln"),
	unary("sin"),
	unary("cos"),
	unary("tan"),
	unary("asin"),
	unary("acos"),
	unary("normalize"),
	unary("magnitude"),
	unary("premult"),
	unary("unpremult"),
	// Two inputs, whose right one MaterialX defaults so that it leaves the left one unchanged.
	binary("add", 0.0),
	binary("subtract", 0.0),
	binary("multiply", 1.0),
	binary("divide", 1.0),
	binary("modulo", 1.0),
	binary("power", 1.0),
	binary("min", 0.0),
	binary("max", 0.0),
	binary("atan2", 1.0),
	node("dotproduct", &[REQUIRED_IN1, REQUIRED_IN2], &[]),
	node("crossproduct", &[REQUIRED_IN1, REQUIRED_IN2], &[]),
	// Ranges and blends.
	node("clamp", &[REQUIRED_IN, value("low", &[0.0]), value("high", &[1.0])], &[]),
	node("smoothstep", &[REQUIRED_IN, value("low", &[0.0]), value("high", &[1.0])], &[]),
	node(
		"mix",
		&[required_input("bg"), required_input("fg"), value("mix", &[0.0])],
		&[],
	),
	node(
		"remap",
		&[
			REQUIRED_IN,
			value("inlow", &[0.0]),
			value("inhigh", &[1.0]),
			value("outlow", &[0.0]),
			value("outhigh", &[1.0]),
		],
		&[],
	),
	node("invert", &[REQUIRED_IN, value("amount", &[1.0])], &[]),
	// Conditions.
	compare("ifgreater"),
	compare("ifgreatereq"),
	compare("ifequal"),
	node("switch", &SWITCH, &[]),
	// Channels.
	node("extract", &[REQUIRED_IN], &[optional("index", SettingKind::Index)]),
	node("swizzle", &[REQUIRED_IN], &[required("channels", SettingKind::Channels)]),
	node("combine", &COMBINE, &[]),
	node("convert", &[REQUIRED_IN], &[required("type", SettingKind::Type)]),
	node("luminance", &[REQUIRED_IN], &[optional("lumacoeffs", SettingKind::Value)]),
	// Normals.
	node(
		"normalmap",
		&[
			REQUIRED_IN,
			value("scale", &[1.0]),
			unset("normal"),
			unset("tangent"),
			unset("bitangent"),
		],
		&[],
	),
	// The surface the graph ends in.
	NodeKind {
		name: "surface",
		inputs: &[
			unset("base_color"),
			unset("opacity"),
			unset("metalness"),
			unset("roughness"),
			unset("normal"),
			unset("emission"),
			unset("occlusion"),
			unset("transmission"),
			unset("translucency"),
			unset("subsurface_radius"),
			unset("thickness"),
		],
		settings: &[],
		produces: false,
	},
];

const REQUIRED_IN: Input = required_input("in");
const REQUIRED_IN1: Input = required_input("in1");
const REQUIRED_IN2: Input = required_input("in2");

/// `switch`'s inputs: the selector, then its numbered inputs, every one optional.
const SWITCH: [Input; SWITCH_INPUTS + 1] = {
	let mut inputs = [value("which", &[0.0]); SWITCH_INPUTS + 1];
	let mut index = 0;
	while index < SWITCH_INPUTS {
		inputs[index + 1] = unset(NUMBERED_INPUTS[index]);
		index += 1;
	}
	inputs
};

/// `combine`'s numbered inputs, every one optional, read until the first one a file leaves out.
const COMBINE: [Input; COMBINE_INPUTS] = {
	let mut inputs = [unset(""); COMBINE_INPUTS];
	let mut index = 0;
	while index < COMBINE_INPUTS {
		inputs[index] = unset(NUMBERED_INPUTS[index]);
		index += 1;
	}
	inputs
};

const fn node(name: &'static str, inputs: &'static [Input], settings: &'static [Setting]) -> NodeKind {
	NodeKind {
		name,
		inputs,
		settings,
		produces: true,
	}
}

const fn unary(name: &'static str) -> NodeKind {
	node(name, &[REQUIRED_IN], &[])
}

/// A two-input operator whose right input, when left out, leaves the left one unchanged: `in2` reads 0 for `add`
/// and its kin, and 1 for `multiply` and its kin.
const fn binary(name: &'static str, right: f32) -> NodeKind {
	node(name, if right == 0.0 { &BINARY_RIGHT_ZERO } else { &BINARY_RIGHT_ONE }, &[])
}

const BINARY_RIGHT_ZERO: [Input; 2] = [REQUIRED_IN1, value("in2", &[0.0])];
const BINARY_RIGHT_ONE: [Input; 2] = [REQUIRED_IN1, value("in2", &[1.0])];

/// A comparison, whose defaults pass the test and pick 0.
const fn compare(name: &'static str) -> NodeKind {
	node(name, &COMPARE, &[])
}

const COMPARE: [Input; 4] = [
	value("value1", &[1.0]),
	value("value2", &[0.0]),
	value("in1", &[0.0]),
	value("in2", &[0.0]),
];

const fn required_input(name: &'static str) -> Input {
	Input {
		name,
		default: InputDefault::Required,
	}
}

const fn unset(name: &'static str) -> Input {
	Input {
		name,
		default: InputDefault::Unset,
	}
}

const fn value(name: &'static str, default: &'static [f32]) -> Input {
	Input {
		name,
		default: InputDefault::Value(default),
	}
}

const fn required(name: &'static str, kind: SettingKind) -> Setting {
	Setting {
		name,
		kind,
		required: true,
	}
}

const fn optional(name: &'static str, kind: SettingKind) -> Setting {
	Setting {
		name,
		kind,
		required: false,
	}
}
