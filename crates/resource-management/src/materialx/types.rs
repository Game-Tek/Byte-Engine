//! Model the MaterialX type system and the text form of its values.
//!
//! MaterialX is strongly typed: every port and every value carries a [`DataType`], and values are
//! written as text inside XML attributes. This module turns that text into [`Value`], which the
//! [`Dag`](super::Dag) hands to whatever consumes the graph.

use std::{
	borrow::Cow,
	fmt::{Display, Formatter},
};

use super::{Alloc, error::ValueError};

/// The `Version` struct records which MaterialX specification a document was written against.
///
/// Compare it against [`SUPPORTED_VERSION`](super::SUPPORTED_VERSION) before relying on newer element semantics.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Version {
	pub major: u32,
	pub minor: u32,
}

impl Version {
	pub const fn new(major: u32, minor: u32) -> Self {
		Version { major, minor }
	}

	/// Reads a `major.minor` version string, accepting a bare major number as `major.0`.
	pub fn parse(text: &str) -> Option<Self> {
		let text = text.trim();

		let (major, minor) = match text.split_once('.') {
			Some((major, minor)) => (major, minor),
			None => (text, "0"),
		};

		Some(Version {
			major: major.parse().ok()?,
			minor: minor.parse().ok()?,
		})
	}
}

impl Display for Version {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}.{}", self.major, self.minor)
	}
}

/// The `TypeSemantic` enum records how a MaterialX type is meant to be interpreted and connected.
///
/// Semantics come from `<typedef>` elements; look one up with [`Document::semantic`](super::Document::semantic).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypeSemantic<'a> {
	/// Plain data with no special handling.
	Default,
	/// A color, which is subject to color space conversion.
	Color,
	/// A shader output, which may drive a material input.
	Shader,
	/// A material output, which a look may assign to geometry.
	Material,
	/// A semantic this parser does not know, kept as written.
	Other(&'a str),
}

impl<'a> TypeSemantic<'a> {
	/// Reads the `semantic` attribute of a `<typedef>`.
	pub fn parse(text: &'a str) -> Self {
		match text {
			"default" => TypeSemantic::Default,
			"color" => TypeSemantic::Color,
			"shader" => TypeSemantic::Shader,
			"material" => TypeSemantic::Material,
			other => TypeSemantic::Other(other),
		}
	}
}

/// The `DataType` enum identifies the type of a MaterialX value, port, or data stream.
///
/// Standard types are named variants; anything declared by a `<typedef>`, such as the `BSDF`, `EDF`
/// and `VDF` closures of the physically based shading library, arrives as [`DataType::Custom`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DataType<'a> {
	Integer,
	Boolean,
	Float,
	Color3,
	Color4,
	Vector2,
	Vector3,
	Vector4,
	Matrix33,
	Matrix44,
	String,
	Filename,
	GeomName,
	IntegerArray,
	FloatArray,
	Color3Array,
	Color4Array,
	Vector2Array,
	Vector3Array,
	Vector4Array,
	StringArray,
	GeomNameArray,
	SurfaceShader,
	DisplacementShader,
	VolumeShader,
	LightShader,
	Material,
	/// The placeholder type a node instance declares when its declaration has more than one output.
	MultiOutput,
	/// The explicit "no type" of the standard library.
	None,
	/// A type introduced by a `<typedef>`.
	Custom(&'a str),
}

impl<'a> DataType<'a> {
	/// Reads a `type` attribute, keeping unknown names as [`DataType::Custom`] so custom types survive.
	pub fn parse(text: &'a str) -> Self {
		match text {
			"integer" => DataType::Integer,
			"boolean" => DataType::Boolean,
			"float" => DataType::Float,
			"color3" => DataType::Color3,
			"color4" => DataType::Color4,
			"vector2" => DataType::Vector2,
			"vector3" => DataType::Vector3,
			"vector4" => DataType::Vector4,
			"matrix33" => DataType::Matrix33,
			"matrix44" => DataType::Matrix44,
			"string" => DataType::String,
			"filename" => DataType::Filename,
			"geomname" => DataType::GeomName,
			"integerarray" => DataType::IntegerArray,
			"floatarray" => DataType::FloatArray,
			"color3array" => DataType::Color3Array,
			"color4array" => DataType::Color4Array,
			"vector2array" => DataType::Vector2Array,
			"vector3array" => DataType::Vector3Array,
			"vector4array" => DataType::Vector4Array,
			"stringarray" => DataType::StringArray,
			"geomnamearray" => DataType::GeomNameArray,
			"surfaceshader" => DataType::SurfaceShader,
			"displacementshader" => DataType::DisplacementShader,
			"volumeshader" => DataType::VolumeShader,
			"lightshader" => DataType::LightShader,
			"material" => DataType::Material,
			"multioutput" => DataType::MultiOutput,
			"none" => DataType::None,
			other => DataType::Custom(other),
		}
	}

	/// Returns the name this type is written with in a `type` attribute.
	pub fn name(&self) -> &'a str {
		match self {
			DataType::Integer => "integer",
			DataType::Boolean => "boolean",
			DataType::Float => "float",
			DataType::Color3 => "color3",
			DataType::Color4 => "color4",
			DataType::Vector2 => "vector2",
			DataType::Vector3 => "vector3",
			DataType::Vector4 => "vector4",
			DataType::Matrix33 => "matrix33",
			DataType::Matrix44 => "matrix44",
			DataType::String => "string",
			DataType::Filename => "filename",
			DataType::GeomName => "geomname",
			DataType::IntegerArray => "integerarray",
			DataType::FloatArray => "floatarray",
			DataType::Color3Array => "color3array",
			DataType::Color4Array => "color4array",
			DataType::Vector2Array => "vector2array",
			DataType::Vector3Array => "vector3array",
			DataType::Vector4Array => "vector4array",
			DataType::StringArray => "stringarray",
			DataType::GeomNameArray => "geomnamearray",
			DataType::SurfaceShader => "surfaceshader",
			DataType::DisplacementShader => "displacementshader",
			DataType::VolumeShader => "volumeshader",
			DataType::LightShader => "lightshader",
			DataType::Material => "material",
			DataType::MultiOutput => "multioutput",
			DataType::None => "none",
			DataType::Custom(name) => name,
		}
	}

	/// Returns how many float components a value of this type holds, or `None` for non-numeric types.
	pub fn component_count(&self) -> Option<usize> {
		match self {
			DataType::Float => Some(1),
			DataType::Vector2 => Some(2),
			DataType::Color3 | DataType::Vector3 => Some(3),
			DataType::Color4 | DataType::Vector4 => Some(4),
			DataType::Matrix33 => Some(9),
			DataType::Matrix44 => Some(16),
			_ => None,
		}
	}

	/// Returns the semantic the standard library declares for this type, or `None` for custom types.
	///
	/// Custom types carry whatever semantic their `<typedef>` declares, so read it from the
	/// [`Document`](super::Document) instead of assuming a default.
	pub fn standard_semantic(&self) -> Option<TypeSemantic<'a>> {
		match self {
			DataType::Color3 | DataType::Color4 | DataType::Color3Array | DataType::Color4Array => Some(TypeSemantic::Color),
			DataType::SurfaceShader | DataType::DisplacementShader | DataType::VolumeShader | DataType::LightShader => {
				Some(TypeSemantic::Shader)
			}
			DataType::Material => Some(TypeSemantic::Material),
			DataType::Custom(_) => None,
			_ => Some(TypeSemantic::Default),
		}
	}

	/// Returns whether a value of `source` may drive an input of this type.
	///
	/// Types must match, except that MaterialX lets a `string` output feed a `filename` input, and
	/// [`DataType::MultiOutput`] stands in wherever this document carries no declaration to check against.
	pub fn accepts(&self, source: &DataType<'_>) -> bool {
		self == source
			|| matches!((self, source), (DataType::Filename, DataType::String))
			|| matches!(self, DataType::MultiOutput)
			|| matches!(source, DataType::MultiOutput)
	}
}

impl Display for DataType<'_> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.name())
	}
}

/// The `Value` enum holds one MaterialX constant, already converted from the text of an XML attribute.
///
/// Read values off [`Input::source`](super::dag::Input::source) after resolving a document into a
/// [`Dag`](super::Dag).
#[derive(Clone, Debug, PartialEq)]
pub enum Value<'a> {
	Integer(i32),
	Boolean(bool),
	Float(f32),
	Color3([f32; 3]),
	Color4([f32; 4]),
	Vector2([f32; 2]),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
	Matrix33([f32; 9]),
	Matrix44([f32; 16]),
	String(&'a str),
	/// A resource reference, which may still contain `<UDIM>`, `[token]` and `{frame}` substitutions.
	Filename(&'a str),
	GeomName(&'a str),
	IntegerArray(Vec<i32, Alloc<'a>>),
	FloatArray(Vec<f32, Alloc<'a>>),
	Color3Array(Vec<[f32; 3], Alloc<'a>>),
	Color4Array(Vec<[f32; 4], Alloc<'a>>),
	Vector2Array(Vec<[f32; 2], Alloc<'a>>),
	Vector3Array(Vec<[f32; 3], Alloc<'a>>),
	Vector4Array(Vec<[f32; 4], Alloc<'a>>),
	/// String array entries, borrowed unless the document escaped a separator inside one.
	StringArray(Vec<Cow<'a, str>, Alloc<'a>>),
	GeomNameArray(Vec<Cow<'a, str>, Alloc<'a>>),
	/// The members of a custom struct type, each kept as written because member types live in a `<typedef>`.
	Struct(Vec<&'a str, Alloc<'a>>),
	/// A value of a custom or shader-semantic type, kept as written.
	Opaque(&'a str),
}

impl<'a> Value<'a> {
	/// Reads the text of a `value` attribute as a value of `data_type`, drawing lists from `allocator`.
	pub fn parse(data_type: &DataType<'_>, text: &'a str, allocator: Alloc<'a>) -> Result<Self, ValueError> {
		match data_type {
			DataType::Integer => Ok(Value::Integer(parse_integer(text)?)),
			DataType::Boolean => Ok(Value::Boolean(parse_boolean(text)?)),
			DataType::Float => Ok(Value::Float(parse_components::<1>(text)?[0])),
			DataType::Color3 => Ok(Value::Color3(parse_components(text)?)),
			DataType::Color4 => Ok(Value::Color4(parse_components(text)?)),
			DataType::Vector2 => Ok(Value::Vector2(parse_components(text)?)),
			DataType::Vector3 => Ok(Value::Vector3(parse_components(text)?)),
			DataType::Vector4 => Ok(Value::Vector4(parse_components(text)?)),
			DataType::Matrix33 => Ok(Value::Matrix33(parse_components(text)?)),
			DataType::Matrix44 => Ok(Value::Matrix44(parse_components(text)?)),
			DataType::String => Ok(Value::String(text)),
			DataType::Filename => Ok(Value::Filename(text)),
			DataType::GeomName => Ok(Value::GeomName(text)),
			DataType::IntegerArray => parse_list(text, allocator, parse_integer).map(Value::IntegerArray),
			DataType::FloatArray => parse_list(text, allocator, parse_float).map(Value::FloatArray),
			DataType::Color3Array => parse_vector_list::<3>(text, allocator).map(Value::Color3Array),
			DataType::Color4Array => parse_vector_list::<4>(text, allocator).map(Value::Color4Array),
			DataType::Vector2Array => parse_vector_list::<2>(text, allocator).map(Value::Vector2Array),
			DataType::Vector3Array => parse_vector_list::<3>(text, allocator).map(Value::Vector3Array),
			DataType::Vector4Array => parse_vector_list::<4>(text, allocator).map(Value::Vector4Array),
			DataType::StringArray => Ok(Value::StringArray(parse_string_list(text, allocator))),
			DataType::GeomNameArray => Ok(Value::GeomNameArray(parse_string_list(text, allocator))),
			// Shader- and material-semantic inputs only ever carry "" to mean "nothing connected".
			DataType::SurfaceShader
			| DataType::DisplacementShader
			| DataType::VolumeShader
			| DataType::LightShader
			| DataType::Material
			| DataType::MultiOutput
			| DataType::None => Ok(Value::Opaque(text)),
			DataType::Custom(_) => parse_custom(text, allocator),
		}
	}

	/// Returns the value MaterialX assumes for an input of `data_type` that carries neither a value nor a connection.
	///
	/// Arrays default to empty because their length comes from the node's own declaration.
	pub fn default_for(data_type: &DataType<'_>, allocator: Alloc<'a>) -> Self {
		match data_type {
			DataType::Integer => Value::Integer(0),
			DataType::Boolean => Value::Boolean(false),
			DataType::Float => Value::Float(0.0),
			DataType::Color3 => Value::Color3([0.0; 3]),
			DataType::Color4 => Value::Color4([0.0; 4]),
			DataType::Vector2 => Value::Vector2([0.0; 2]),
			DataType::Vector3 => Value::Vector3([0.0; 3]),
			DataType::Vector4 => Value::Vector4([0.0; 4]),
			DataType::Matrix33 => Value::Matrix33(IDENTITY_MATRIX33),
			DataType::Matrix44 => Value::Matrix44(IDENTITY_MATRIX44),
			DataType::String => Value::String(""),
			DataType::Filename => Value::Filename(""),
			DataType::GeomName => Value::GeomName(""),
			DataType::IntegerArray => Value::IntegerArray(Vec::new_in(allocator)),
			DataType::FloatArray => Value::FloatArray(Vec::new_in(allocator)),
			DataType::Color3Array => Value::Color3Array(Vec::new_in(allocator)),
			DataType::Color4Array => Value::Color4Array(Vec::new_in(allocator)),
			DataType::Vector2Array => Value::Vector2Array(Vec::new_in(allocator)),
			DataType::Vector3Array => Value::Vector3Array(Vec::new_in(allocator)),
			DataType::Vector4Array => Value::Vector4Array(Vec::new_in(allocator)),
			DataType::StringArray => Value::StringArray(Vec::new_in(allocator)),
			DataType::GeomNameArray => Value::GeomNameArray(Vec::new_in(allocator)),
			_ => Value::Opaque(""),
		}
	}

	/// Returns this value's float components when it holds a scalar, vector, color, or matrix.
	pub fn components(&self) -> Option<&[f32]> {
		match self {
			Value::Float(value) => Some(std::slice::from_ref(value)),
			Value::Color3(value) | Value::Vector3(value) => Some(value),
			Value::Color4(value) | Value::Vector4(value) => Some(value),
			Value::Vector2(value) => Some(value),
			Value::Matrix33(value) => Some(value),
			Value::Matrix44(value) => Some(value),
			_ => None,
		}
	}
}

const IDENTITY_MATRIX33: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

const IDENTITY_MATRIX44: [f32; 16] = [
	1.0, 0.0, 0.0, 0.0, //
	0.0, 1.0, 0.0, 0.0, //
	0.0, 0.0, 1.0, 0.0, //
	0.0, 0.0, 0.0, 1.0,
];

/// Splits a numeric value list on commas, whitespace, or both.
///
/// The specification writes these lists with commas, but documents in the wild separate components
/// with spaces alone, so both are accepted.
fn split_values(text: &str) -> impl Iterator<Item = &str> {
	text.split([',', ' ', '\t', '\r', '\n']).filter(|field| !field.is_empty())
}

fn parse_integer(text: &str) -> Result<i32, ValueError> {
	text.trim().parse().map_err(|_| ValueError::Number {
		text: text.trim().to_string(),
	})
}

fn parse_float(text: &str) -> Result<f32, ValueError> {
	text.trim().parse().map_err(|_| ValueError::Number {
		text: text.trim().to_string(),
	})
}

fn parse_boolean(text: &str) -> Result<bool, ValueError> {
	match text.trim() {
		"true" => Ok(true),
		"false" => Ok(false),
		other => Err(ValueError::Boolean { text: other.to_string() }),
	}
}

/// Reads exactly `N` comma-separated floats, which is how MaterialX writes vectors, colors, and matrices.
fn parse_components<const N: usize>(text: &str) -> Result<[f32; N], ValueError> {
	let mut components = [0.0f32; N];
	let mut count = 0usize;

	for field in split_values(text) {
		let component = parse_float(field)?;

		if count < N {
			components[count] = component;
		}

		count += 1;
	}

	if count == N {
		Ok(components)
	} else {
		Err(ValueError::ComponentCount {
			expected: N,
			found: count,
		})
	}
}

/// Reads a list of numbers into `allocator`, treating text with nothing in it as an empty list.
fn parse_list<'a, T>(
	text: &str,
	allocator: Alloc<'a>,
	parse: impl Fn(&str) -> Result<T, ValueError>,
) -> Result<Vec<T, Alloc<'a>>, ValueError> {
	let mut values = Vec::new_in(allocator);

	for field in split_values(text) {
		values.push(parse(field)?);
	}

	Ok(values)
}

/// Reads a flat float list as a list of `N`-component elements, which is how MaterialX writes vector arrays.
fn parse_vector_list<'a, const N: usize>(text: &str, allocator: Alloc<'a>) -> Result<Vec<[f32; N], Alloc<'a>>, ValueError> {
	let flat = parse_list(text, allocator, parse_float)?;

	if flat.len() % N != 0 {
		return Err(ValueError::ArrayLength {
			stride: N,
			found: flat.len(),
		});
	}

	let mut elements = Vec::with_capacity_in(flat.len() / N, allocator);

	elements.extend_from_slice(flat.as_chunks::<N>().0);

	Ok(elements)
}

/// Reads a string array, honouring the MaterialX convention that `\,`, `\;` and `\\` are literal characters.
///
/// Entries are borrowed from `text`; only an entry that actually carries an escape needs a string of
/// its own.
fn parse_string_list<'a>(text: &'a str, allocator: Alloc<'a>) -> Vec<Cow<'a, str>, Alloc<'a>> {
	let mut entries = Vec::new_in(allocator);

	if text.trim().is_empty() {
		return entries;
	}

	let mut start = 0usize;
	let mut escaped = false;

	// Separators and escapes are ASCII, so scanning bytes never splits a character.
	for (index, byte) in text.bytes().enumerate() {
		match (escaped, byte) {
			(true, _) => escaped = false,
			(false, b'\\') => escaped = true,
			(false, b',') => {
				entries.push(unescape(&text[start..index]));
				start = index + 1;
			}
			(false, _) => {}
		}
	}

	entries.push(unescape(&text[start..]));

	entries
}

/// Removes the backslash escapes MaterialX allows inside one string array entry.
fn unescape(entry: &str) -> Cow<'_, str> {
	let entry = entry.trim();

	if !entry.contains('\\') {
		return Cow::Borrowed(entry);
	}

	let mut unescaped = String::with_capacity(entry.len());
	let mut escaped = false;

	for character in entry.chars() {
		match (escaped, character) {
			(true, _) => {
				unescaped.push(character);
				escaped = false;
			}
			(false, '\\') => escaped = true,
			(false, _) => unescaped.push(character),
		}
	}

	Cow::Owned(unescaped)
}

/// Reads a value of a `<typedef>` type: a brace-enclosed struct initialiser, or opaque text.
fn parse_custom<'a>(text: &'a str, allocator: Alloc<'a>) -> Result<Value<'a>, ValueError> {
	let trimmed = text.trim();

	let Some(body) = trimmed.strip_prefix('{') else {
		return Ok(Value::Opaque(text));
	};

	let malformed = || ValueError::Struct {
		text: trimmed.to_string(),
	};

	let body = body.strip_suffix('}').ok_or_else(malformed)?;

	// Members are separated by semicolons, and a member may itself be a nested struct. Braces and
	// semicolons are ASCII, so scanning bytes never splits a character.
	let mut members = Vec::new_in(allocator);
	let mut start = 0usize;
	let mut depth = 0usize;

	for (index, byte) in body.bytes().enumerate() {
		match byte {
			b'{' => depth += 1,
			b'}' => depth = depth.checked_sub(1).ok_or_else(malformed)?,
			b';' if depth == 0 => {
				members.push(body[start..index].trim());
				start = index + 1;
			}
			_ => {}
		}
	}

	if depth != 0 {
		return Err(malformed());
	}

	members.push(body[start..].trim());

	Ok(Value::Struct(members))
}

#[cfg(test)]
mod tests {
	use std::{alloc::Global, borrow::Cow};

	use super::{DataType, Value, Version};
	use crate::materialx::error::ValueError;

	/// Reads a value with the global allocator, which is all these tests need.
	fn parse<'a>(data_type: &DataType<'_>, text: &'a str) -> Result<Value<'a>, ValueError> {
		Value::parse(data_type, text, &Global)
	}

	/// Builds an expected list in the same shape the parser produces.
	fn list<T>(values: impl IntoIterator<Item = T>) -> Vec<T, super::Alloc<'static>> {
		let mut list: Vec<T, super::Alloc<'static>> = Vec::new_in(&Global);

		list.extend(values);

		list
	}

	#[test]
	fn reads_document_versions() {
		assert_eq!(Version::parse("1.39"), Some(Version::new(1, 39)));
		assert_eq!(Version::parse(" 1 "), Some(Version::new(1, 0)));
		assert_eq!(Version::parse("1.39.4"), None);
	}

	#[test]
	fn reads_scalar_and_vector_values() {
		assert_eq!(parse(&DataType::Float, "1.5"), Ok(Value::Float(1.5)));
		assert_eq!(parse(&DataType::Integer, " 7 "), Ok(Value::Integer(7)));
		assert_eq!(parse(&DataType::Boolean, "true"), Ok(Value::Boolean(true)));
		assert_eq!(parse(&DataType::Color3, "0.1,0.2, 0.3"), Ok(Value::Color3([0.1, 0.2, 0.3])));
		assert_eq!(parse(&DataType::Vector2, "0.234,0.885"), Ok(Value::Vector2([0.234, 0.885])));
		assert_eq!(
			parse(&DataType::Matrix33, "1,0,0, 0,1,0, 0,0,1"),
			Ok(Value::Matrix33([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]))
		);
	}

	#[test]
	fn reports_the_component_count_actually_written() {
		assert_eq!(
			parse(&DataType::Color3, "0.1,0.2"),
			Err(ValueError::ComponentCount { expected: 3, found: 2 })
		);
		assert_eq!(
			parse(&DataType::Color3, "0.1,0.2,0.3,0.4"),
			Err(ValueError::ComponentCount { expected: 3, found: 4 })
		);
	}

	#[test]
	fn reads_array_values() {
		assert_eq!(
			parse(&DataType::IntegerArray, "1,2,3"),
			Ok(Value::IntegerArray(list([1, 2, 3])))
		);
		assert_eq!(
			parse(&DataType::Vector2Array, "0,.1, .4,.5"),
			Ok(Value::Vector2Array(list([[0.0, 0.1], [0.4, 0.5]])))
		);
		assert_eq!(
			parse(&DataType::Vector3Array, "1,2"),
			Err(ValueError::ArrayLength { stride: 3, found: 2 })
		);
		assert_eq!(parse(&DataType::FloatArray, ""), Ok(Value::FloatArray(list([]))));
	}

	#[test]
	fn reads_escaped_string_arrays() {
		assert_eq!(
			parse(&DataType::StringArray, r"hello, there\, world, \\"),
			Ok(Value::StringArray(list([
				Cow::Borrowed("hello"),
				Cow::Owned("there, world".to_string()),
				Cow::Owned(r"\".to_string()),
			])))
		);
	}

	#[test]
	fn reads_struct_values_with_nested_members() {
		assert_eq!(
			parse(
				&DataType::Custom("exampletype"),
				"{3; 0.18,0.2,0.11; foo,bar; {0.0,1.0}; 3.4,5.1}"
			),
			Ok(Value::Struct(list(["3", "0.18,0.2,0.11", "foo,bar", "{0.0,1.0}", "3.4,5.1"])))
		);
	}

	#[test]
	fn keeps_unbraced_custom_values_as_written() {
		assert_eq!(parse(&DataType::Custom("BSDF"), ""), Ok(Value::Opaque("")));
	}

	#[test]
	fn defaults_matrices_to_the_identity() {
		assert_eq!(
			Value::default_for(&DataType::Matrix33, &Global),
			Value::Matrix33([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
		);
	}

	#[test]
	fn strings_may_drive_filename_inputs_but_not_the_other_way_round() {
		assert!(DataType::Filename.accepts(&DataType::String));
		assert!(!DataType::String.accepts(&DataType::Filename));
		assert!(DataType::Color3.accepts(&DataType::Color3));
		assert!(!DataType::Color3.accepts(&DataType::Vector3));
	}
}
