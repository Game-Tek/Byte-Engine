use super::*;

pub(crate) fn parse_literal(value: &str, value_type: &ValueType) -> Result<Value, VmError> {
	let parsed = match value_type {
		ValueType::Bool => match value {
			"true" => Value::Bool(true),
			"false" => Value::Bool(false),
			_ => {
				return Err(VmError::InvalidLiteral {
					value: value.to_string(),
					value_type: value_type.name().to_string(),
				});
			}
		},
		ValueType::U8 => value.parse::<u8>().map(Value::U8).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::U16 => value.parse::<u16>().map(Value::U16).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::U32 => value.parse::<u32>().map(Value::U32).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::I32 => value.parse::<i32>().map(Value::I32).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::F16 => value
			.parse::<f32>()
			.map(|value| Value::F16(f16::from_f32(value)))
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::Vec2U16 | ValueType::Vec4U16 | ValueType::Vec2I | ValueType::Vec2U | ValueType::Vec3U | ValueType::Vec4U => {
			return Err(VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			});
		}
		ValueType::F32 => value.parse::<f32>().map(Value::F32).map_err(|_| VmError::InvalidLiteral {
			value: value.to_string(),
			value_type: value_type.name().to_string(),
		})?,
		ValueType::Vec2F16
		| ValueType::Vec3F16
		| ValueType::Vec4F16
		| ValueType::Vec2F
		| ValueType::Vec3F
		| ValueType::Vec4F
		| ValueType::PackedVec4F
		| ValueType::Mat4F
		| ValueType::Mat4x3F
		| ValueType::Texture2D
		| ValueType::Texture3D
		| ValueType::TextureCube
		| ValueType::TextureCubeArray
		| ValueType::ArrayTexture2D
		| ValueType::Struct { .. } => {
			return Err(VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			});
		}
	};

	Ok(parsed)
}

pub(crate) fn construct_value(value_type: &ValueType, components: &[Value]) -> Result<Value, VmError> {
	match value_type {
		ValueType::Vec2U16 => Ok(Value::Vec2U16(extract_u16_components::<2>(components)?)),
		ValueType::Vec4U16 => Ok(Value::Vec4U16(extract_u16_components::<4>(components)?)),
		ValueType::Vec2I => Ok(Value::Vec2I(extract_i32_components::<2>(components)?)),
		ValueType::Vec2U => Ok(Value::Vec2U(extract_u32_components::<2>(components)?)),
		ValueType::Vec3U => Ok(Value::Vec3U(extract_u32_components::<3>(components)?)),
		ValueType::Vec4U => Ok(Value::Vec4U(extract_u32_components::<4>(components)?)),
		ValueType::Vec2F16 => Ok(Value::Vec2F16(extract_f16_components::<2>(components)?)),
		ValueType::Vec3F16 => Ok(Value::Vec3F16(extract_f16_components::<3>(components)?)),
		ValueType::Vec4F16 => Ok(Value::Vec4F16(extract_f16_components::<4>(components)?)),
		ValueType::Vec2F => Ok(Value::Vec2F(extract_f32_components::<2>(components)?)),
		ValueType::Vec3F => Ok(Value::Vec3F(extract_f32_components::<3>(components)?)),
		ValueType::Vec4F => Ok(Value::Vec4F(extract_f32_components::<4>(components)?)),
		ValueType::PackedVec4F => Ok(Value::PackedVec4F(extract_f32_components::<4>(components)?)),
		ValueType::Mat4F => Ok(Value::Mat4F(extract_f32_components::<16>(components)?)),
		ValueType::Mat4x3F => Ok(Value::Mat4x3F(extract_f32_components::<12>(components)?)),
		ValueType::Struct { fields, .. } => {
			if fields.len() != components.len()
				|| !components
					.iter()
					.zip(fields)
					.all(|(component, field)| component.matches_type(field.value_type()))
			{
				return Err(VmError::TypeMismatch {
					expected: value_type.name().to_string(),
					found: "constructor fields".to_string(),
				});
			}
			Ok(Value::Struct {
				value_type: value_type.clone(),
				fields: components.to_vec(),
			})
		}
		_ => Err(VmError::UnsupportedExpression {
			message: format!("`{}` is not a constructor-backed VM value type", value_type.name()),
		}),
	}
}

/// The `Lane` trait lets one generic reader, writer, and constructor handle every scalar lane type a VM vector stores.
/// Implement it for a scalar type before adding a vector value built from that scalar.
pub(crate) trait Lane: Copy + Default {
	/// The number of bytes one lane occupies in packed VM memory.
	const SIZE: usize;
	/// The BESL type name that constructor errors report.
	const NAME: &'static str;

	/// Decodes one lane from exactly [`Self::SIZE`] native-endian bytes.
	fn from_ne(bytes: &[u8]) -> Self;
	/// Encodes one lane into exactly [`Self::SIZE`] native-endian bytes.
	fn to_ne(self, bytes: &mut [u8]);
}

/// Implements [`Lane`] for primitive types whose byte conversion is `from_ne_bytes`/`to_ne_bytes`.
macro_rules! primitive_lane {
	($($type:ty),*) => {$(
		impl Lane for $type {
			const SIZE: usize = size_of::<$type>();
			const NAME: &'static str = stringify!($type);

			fn from_ne(bytes: &[u8]) -> Self {
				<$type>::from_ne_bytes(bytes.try_into().expect("Invalid lane byte count"))
			}

			fn to_ne(self, bytes: &mut [u8]) {
				bytes.copy_from_slice(&self.to_ne_bytes());
			}
		}
	)*};
}

primitive_lane!(u8, u16, u32, i32, f32);

impl Lane for f16 {
	const SIZE: usize = 2;
	const NAME: &'static str = "f16";

	fn from_ne(bytes: &[u8]) -> Self {
		f16::from_bits(u16::from_ne(bytes))
	}

	fn to_ne(self, bytes: &mut [u8]) {
		self.to_bits().to_ne(bytes);
	}
}

/// Decodes `N` packed lanes. Callers slice exactly `N * T::SIZE` bytes from a buffer, as [`Buffer`] reads do.
pub(crate) fn read_lanes<T: Lane, const N: usize>(bytes: &[u8]) -> [T; N] {
	debug_assert_eq!(bytes.len(), N * T::SIZE, "Lane reads must receive exactly one value's bytes");
	std::array::from_fn(|index| T::from_ne(&bytes[index * T::SIZE..(index + 1) * T::SIZE]))
}

/// Encodes packed lanes at `offset` after checking the whole destination range once.
pub(crate) fn write_lanes<T: Lane>(buffer: &mut Buffer, offset: usize, values: &[T]) -> Result<(), VmError> {
	let bytes = buffer.bytes_mut(offset, values.len() * T::SIZE)?;
	for (chunk, value) in bytes.chunks_exact_mut(T::SIZE).zip(values) {
		value.to_ne(chunk);
	}
	Ok(())
}

/// Copies `source` lanes into the front of `lanes` through `convert` and returns how many it wrote.
fn fill_lanes<S: Copy, T>(lanes: &mut [T; MAX_COMPONENT_LANES], source: &[S], convert: impl Fn(S) -> T) -> usize {
	for (destination, source) in lanes.iter_mut().zip(source) {
		*destination = convert(*source);
	}
	source.len()
}

/// The widest constructor component, a `mat4f`, spans this many lanes.
const MAX_COMPONENT_LANES: usize = 16;

/// Flattens constructor components into exactly `N` lanes.
///
/// `lanes` writes one accepted component's lanes into scratch storage and returns their count. It returns `None` to
/// reject the component's type, which reports `expected` as the accepted types.
fn extract_components<T: Lane, const N: usize>(
	components: &[Value],
	expected: &str,
	lanes: impl Fn(&Value, &mut [T; MAX_COMPONENT_LANES]) -> Option<usize>,
) -> Result<[T; N], VmError> {
	let mut values = [T::default(); N];
	let mut index = 0;
	for component in components {
		let mut scratch = [T::default(); MAX_COMPONENT_LANES];
		let count = lanes(component, &mut scratch).ok_or_else(|| VmError::TypeMismatch {
			expected: expected.to_string(),
			found: component.value_type().name().to_string(),
		})?;
		if index + count > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} {} components", N, T::NAME),
			});
		}
		values[index..index + count].copy_from_slice(&scratch[..count]);
		index += count;
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} {} components, but found {}", N, T::NAME, index),
		});
	}
	Ok(values)
}

pub(crate) fn extract_f32_components<const N: usize>(components: &[Value]) -> Result<[f32; N], VmError> {
	extract_components(components, "f16 or f32", |component, lanes| {
		Some(match component {
			Value::F16(value) => fill_lanes(lanes, std::slice::from_ref(value), f16::to_f32),
			Value::F32(value) => fill_lanes(lanes, std::slice::from_ref(value), f32::from),
			Value::Vec2F16(value) => fill_lanes(lanes, value, f16::to_f32),
			Value::Vec3F16(value) => fill_lanes(lanes, value, f16::to_f32),
			Value::Vec4F16(value) => fill_lanes(lanes, value, f16::to_f32),
			Value::Vec2F(value) => fill_lanes(lanes, value, f32::from),
			Value::Vec3F(value) => fill_lanes(lanes, value, f32::from),
			Value::Vec4F(value) | Value::PackedVec4F(value) => fill_lanes(lanes, value, f32::from),
			Value::Mat4F(value) => fill_lanes(lanes, value, f32::from),
			Value::Mat4x3F(value) => fill_lanes(lanes, value, f32::from),
			_ => return None,
		})
	})
}

pub(crate) fn extract_f16_components<const N: usize>(components: &[Value]) -> Result<[f16; N], VmError> {
	extract_components(components, "f16 or f32", |component, lanes| {
		Some(match component {
			Value::F16(value) => fill_lanes(lanes, std::slice::from_ref(value), f16::from),
			Value::F32(value) => fill_lanes(lanes, std::slice::from_ref(value), f16::from_f32),
			Value::Vec2F16(value) => fill_lanes(lanes, value, f16::from),
			Value::Vec3F16(value) => fill_lanes(lanes, value, f16::from),
			Value::Vec4F16(value) => fill_lanes(lanes, value, f16::from),
			Value::Vec2F(value) => fill_lanes(lanes, value, f16::from_f32),
			Value::Vec3F(value) => fill_lanes(lanes, value, f16::from_f32),
			Value::Vec4F(value) => fill_lanes(lanes, value, f16::from_f32),
			_ => return None,
		})
	})
}

pub(crate) fn extract_u32_components<const N: usize>(components: &[Value]) -> Result<[u32; N], VmError> {
	extract_components(components, ValueType::U32.name(), |component, lanes| {
		Some(match component {
			Value::U32(value) => fill_lanes(lanes, std::slice::from_ref(value), u32::from),
			Value::Vec2U(value) => fill_lanes(lanes, value, u32::from),
			Value::Vec3U(value) => fill_lanes(lanes, value, u32::from),
			Value::Vec4U(value) => fill_lanes(lanes, value, u32::from),
			_ => return None,
		})
	})
}

pub(crate) fn extract_u16_components<const N: usize>(components: &[Value]) -> Result<[u16; N], VmError> {
	// `u32` components narrow to their low 16 bits, matching an `as u16` cast.
	extract_components(components, "u16 or u32", |component, lanes| {
		Some(match component {
			Value::U16(value) => fill_lanes(lanes, std::slice::from_ref(value), u16::from),
			Value::U32(value) => fill_lanes(lanes, std::slice::from_ref(value), |value| value as u16),
			Value::Vec2U16(value) => fill_lanes(lanes, value, u16::from),
			Value::Vec4U16(value) => fill_lanes(lanes, value, u16::from),
			Value::Vec2U(value) => fill_lanes(lanes, value, |value| value as u16),
			Value::Vec3U(value) => fill_lanes(lanes, value, |value| value as u16),
			Value::Vec4U(value) => fill_lanes(lanes, value, |value| value as u16),
			_ => return None,
		})
	})
}

pub(crate) fn extract_i32_components<const N: usize>(components: &[Value]) -> Result<[i32; N], VmError> {
	extract_components(components, ValueType::I32.name(), |component, lanes| {
		Some(match component {
			Value::I32(value) => fill_lanes(lanes, std::slice::from_ref(value), i32::from),
			Value::Vec2I(value) => fill_lanes(lanes, value, i32::from),
			_ => return None,
		})
	})
}
