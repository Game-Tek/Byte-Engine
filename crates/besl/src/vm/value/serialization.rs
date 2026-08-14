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

pub(crate) fn extract_f32_components<const N: usize>(components: &[Value]) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	let mut index = 0;
	for component in components {
		let component_count = match component {
			Value::F16(_) | Value::F32(_) => 1,
			Value::Vec2F16(value) => value.len(),
			Value::Vec3F16(value) => value.len(),
			Value::Vec4F16(value) => value.len(),
			Value::Vec2F(value) => value.len(),
			Value::Vec3F(value) => value.len(),
			Value::Vec4F(value) => value.len(),
			Value::PackedVec4F(value) => value.len(),
			Value::Mat4F(value) => value.len(),
			Value::Mat4x3F(value) => value.len(),
			_ => {
				return Err(VmError::TypeMismatch {
					expected: "f16 or f32".to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + component_count > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} f32 components", N),
			});
		}
		match component {
			Value::F16(value) => values[index] = value.to_f32(),
			Value::F32(value) => values[index] = *value,
			Value::Vec2F16(value) => {
				for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
					*destination = source.to_f32();
				}
			}
			Value::Vec3F16(value) => {
				for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
					*destination = source.to_f32();
				}
			}
			Value::Vec4F16(value) => {
				for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
					*destination = source.to_f32();
				}
			}
			Value::Vec2F(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::Vec3F(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::Vec4F(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::PackedVec4F(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::Mat4F(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::Mat4x3F(value) => values[index..index + value.len()].copy_from_slice(value),
			_ => unreachable!("Float constructor components are validated before conversion"),
		}
		index += component_count;
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} f32 components, but found {}", N, index),
		});
	}

	Ok(values)
}

pub(crate) fn extract_f16_components<const N: usize>(components: &[Value]) -> Result<[f16; N], VmError> {
	let mut values = [f16::from_f32(0.0); N];
	let mut index = 0;
	for component in components {
		let component_count = match component {
			Value::F16(_) | Value::F32(_) => 1,
			Value::Vec2F16(value) => value.len(),
			Value::Vec3F16(value) => value.len(),
			Value::Vec4F16(value) => value.len(),
			Value::Vec2F(value) => value.len(),
			Value::Vec3F(value) => value.len(),
			Value::Vec4F(value) => value.len(),
			_ => {
				return Err(VmError::TypeMismatch {
					expected: "f16 or f32".to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + component_count > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} f16 components", N),
			});
		}
		match component {
			Value::F16(value) => values[index] = *value,
			Value::F32(value) => values[index] = f16::from_f32(*value),
			Value::Vec2F16(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::Vec3F16(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::Vec4F16(value) => values[index..index + value.len()].copy_from_slice(value),
			Value::Vec2F(value) => {
				for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
					*destination = f16::from_f32(*source);
				}
			}
			Value::Vec3F(value) => {
				for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
					*destination = f16::from_f32(*source);
				}
			}
			Value::Vec4F(value) => {
				for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
					*destination = f16::from_f32(*source);
				}
			}
			_ => unreachable!("Float constructor components are validated before conversion"),
		}
		index += component_count;
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} f16 components, but found {}", N, index),
		});
	}

	Ok(values)
}

pub(crate) fn extract_u32_components<const N: usize>(components: &[Value]) -> Result<[u32; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let slice: &[u32] = match component {
			Value::U32(value) => std::slice::from_ref(value),
			Value::Vec2U(value) => value,
			Value::Vec3U(value) => value,
			Value::Vec4U(value) => value,
			_ => {
				return Err(VmError::TypeMismatch {
					expected: ValueType::U32.name().to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + slice.len() > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} u32 components", N),
			});
		}
		values[index..index + slice.len()].copy_from_slice(slice);
		index += slice.len();
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} u32 components, but found {}", N, index),
		});
	}

	Ok(values)
}

pub(crate) fn extract_u16_components<const N: usize>(components: &[Value]) -> Result<[u16; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let component_count = match component {
			Value::U16(value) => {
				if index < N {
					values[index] = *value;
				}
				1
			}
			Value::U32(value) => {
				if index < N {
					values[index] = *value as u16;
				}
				1
			}
			Value::Vec2U16(value) => {
				if index + value.len() <= N {
					values[index..index + value.len()].copy_from_slice(value);
				}
				value.len()
			}
			Value::Vec4U16(value) => {
				if index + value.len() <= N {
					values[index..index + value.len()].copy_from_slice(value);
				}
				value.len()
			}
			Value::Vec2U(value) => {
				if index + value.len() <= N {
					for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
						*destination = *source as u16;
					}
				}
				value.len()
			}
			Value::Vec3U(value) => {
				if index + value.len() <= N {
					for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
						*destination = *source as u16;
					}
				}
				value.len()
			}
			Value::Vec4U(value) => {
				if index + value.len() <= N {
					for (destination, source) in values[index..index + value.len()].iter_mut().zip(value) {
						*destination = *source as u16;
					}
				}
				value.len()
			}
			_ => {
				return Err(VmError::TypeMismatch {
					expected: "u16 or u32".to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + component_count > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} u16 components", N),
			});
		}
		index += component_count;
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} u16 components, but found {}", N, index),
		});
	}
	Ok(values)
}

pub(crate) fn extract_i32_components<const N: usize>(components: &[Value]) -> Result<[i32; N], VmError> {
	let mut values = [0; N];
	let mut index = 0;
	for component in components {
		let slice: &[i32] = match component {
			Value::I32(value) => std::slice::from_ref(value),
			Value::Vec2I(value) => value,
			_ => {
				return Err(VmError::TypeMismatch {
					expected: ValueType::I32.name().to_string(),
					found: component.value_type().name().to_string(),
				});
			}
		};
		if index + slice.len() > N {
			return Err(VmError::UnsupportedExpression {
				message: format!("Constructor provides more than {} i32 components", N),
			});
		}
		values[index..index + slice.len()].copy_from_slice(slice);
		index += slice.len();
	}
	if index != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Constructor expected {} i32 components, but found {}", N, index),
		});
	}
	Ok(values)
}

pub(crate) fn read_f16_array<const N: usize>(bytes: &[u8]) -> Result<[f16; N], VmError> {
	if bytes.len() != N * 2 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} f16 values, but found {}", N * 2, N, bytes.len()),
		});
	}

	let mut values = [f16::from_f32(0.0); N];
	for (index, chunk) in bytes.chunks_exact(2).enumerate() {
		values[index] = f16::from_bits(u16::from_ne_bytes(chunk.try_into().expect("Invalid f16 byte count")));
	}
	Ok(values)
}

pub(crate) fn read_f32_array<const N: usize>(bytes: &[u8]) -> Result<[f32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} f32 values, but found {}", N * 4, N, bytes.len()),
		});
	}

	let mut values = [0.0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = f32::from_ne_bytes(chunk.try_into().expect("Invalid f32 byte count"));
	}
	Ok(values)
}

pub(crate) fn read_u32_array<const N: usize>(bytes: &[u8]) -> Result<[u32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} u32 values, but found {}", N * 4, N, bytes.len()),
		});
	}

	let mut values = [0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = u32::from_ne_bytes(chunk.try_into().expect("Invalid u32 byte count"));
	}
	Ok(values)
}

pub(crate) fn read_u16_array<const N: usize>(bytes: &[u8]) -> Result<[u16; N], VmError> {
	if bytes.len() != N * 2 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} u16 values, but found {}", N * 2, N, bytes.len()),
		});
	}
	let mut values = [0; N];
	for (index, chunk) in bytes.chunks_exact(2).enumerate() {
		values[index] = u16::from_ne_bytes(chunk.try_into().expect("Invalid u16 byte count"));
	}
	Ok(values)
}

pub(crate) fn read_i32_array<const N: usize>(bytes: &[u8]) -> Result<[i32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} i32 values, but found {}", N * 4, N, bytes.len()),
		});
	}
	let mut values = [0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = i32::from_ne_bytes(chunk.try_into().expect("Invalid i32 byte count"));
	}
	Ok(values)
}

pub(crate) fn write_f16_slice(buffer: &mut Buffer, offset: usize, values: &[f16]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 2, &value.to_bits().to_ne_bytes())?;
	}
	Ok(())
}

pub(crate) fn write_f32_slice(buffer: &mut Buffer, offset: usize, values: &[f32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

pub(crate) fn write_u32_slice(buffer: &mut Buffer, offset: usize, values: &[u32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}

pub(crate) fn write_u16_slice(buffer: &mut Buffer, offset: usize, values: &[u16]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 2, &value.to_ne_bytes())?;
	}
	Ok(())
}

pub(crate) fn write_i32_slice(buffer: &mut Buffer, offset: usize, values: &[i32]) -> Result<(), VmError> {
	for (index, value) in values.iter().enumerate() {
		buffer.write_bytes(offset + index * 4, &value.to_ne_bytes())?;
	}
	Ok(())
}
