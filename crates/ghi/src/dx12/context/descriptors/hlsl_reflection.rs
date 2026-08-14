use super::super::*;

impl Device {
	/// Applies inferred HLSL structured-buffer strides without overriding explicit metadata.
	pub(crate) fn apply_hlsl_structured_buffer_strides(resources: &mut [ShaderResourceDescriptor], hlsl: &str) {
		let strides = Self::hlsl_structured_buffer_strides(hlsl);
		for resource in resources {
			if !matches!(resource.kind(), ResourceKind::UniformBuffer | ResourceKind::StorageBuffer)
				|| resource.buffer_element_stride() != 4
			{
				continue;
			}
			let Some(stride) = strides.get(&(0, resource.slot().index())).copied() else {
				continue;
			};
			if stride != 0 {
				*resource = resource.buffer_stride(stride);
			}
		}
	}

	/// Extracts structured-buffer element strides from HLSL register declarations.
	pub(crate) fn hlsl_structured_buffer_strides(source: &str) -> HashMap<(u32, u32), u32> {
		let struct_sizes = Self::hlsl_struct_sizes(source);
		let mut strides = HashMap::default();
		let bytes = source.as_bytes();
		let mut index = 0;

		while let Some(relative) = source[index..].find("StructuredBuffer<") {
			let start = index + relative;
			let type_start = start + "StructuredBuffer<".len();
			let Some(type_end_relative) = source[type_start..].find('>') else {
				break;
			};
			let type_end = type_start + type_end_relative;
			let element_type = source[type_start..type_end].trim();
			let Some(stride) = Self::hlsl_type_size(element_type, &struct_sizes) else {
				index = type_end + 1;
				continue;
			};

			let Some(register_relative) = source[type_end..].find("register(") else {
				break;
			};
			let register_start = type_end + register_relative + "register(".len();
			let Some(register_end_relative) = source[register_start..].find(')') else {
				break;
			};
			let register_end = register_start + register_end_relative;
			let register = &source[register_start..register_end];
			if let Some((binding, space)) = Self::hlsl_register_binding(register) {
				strides.insert((space, binding), stride);
			}

			index = register_end + usize::from(register_end < bytes.len());
		}

		strides
	}

	/// Computes byte sizes for HLSL struct declarations used as structured-buffer element types.
	pub(crate) fn hlsl_struct_sizes(source: &str) -> HashMap<String, u32> {
		let mut struct_sizes = HashMap::default();
		let mut index = 0;

		while let Some(relative) = source[index..].find("struct ") {
			let struct_start = index + relative + "struct ".len();
			let name_start = Self::skip_hlsl_whitespace(source, struct_start);
			let name_end = Self::hlsl_identifier_end(source, name_start);
			if name_end == name_start {
				index = struct_start;
				continue;
			}

			let name = source[name_start..name_end].to_string();
			let Some(open_relative) = source[name_end..].find('{') else {
				break;
			};
			let body_start = name_end + open_relative + 1;
			let Some(body_end) = Self::matching_hlsl_brace(source, body_start - 1) else {
				break;
			};

			if let Some(size) = Self::hlsl_struct_body_size(&source[body_start..body_end], &struct_sizes) {
				struct_sizes.insert(name, size);
			}
			index = body_end + 1;
		}

		struct_sizes
	}

	/// Computes a structured-buffer struct body size from field declarations.
	pub(crate) fn hlsl_struct_body_size(body: &str, struct_sizes: &HashMap<String, u32>) -> Option<u32> {
		let mut size = 0u32;
		for statement in body.split(';') {
			let statement = statement.trim();
			if statement.is_empty() || statement.contains('(') {
				continue;
			}
			let mut parts = statement.split_whitespace();
			let Some(field_type) = parts.next() else {
				continue;
			};
			let Some(field_name) = parts.next() else {
				continue;
			};
			let array_count = Self::hlsl_array_count(field_name).unwrap_or(1);
			size = size.checked_add(Self::hlsl_type_size(field_type, struct_sizes)?.checked_mul(array_count)?)?;
		}
		Some(size)
	}

	/// Returns the byte size of a scalar, vector, matrix, or previously parsed struct type.
	pub(crate) fn hlsl_type_size(r#type: &str, struct_sizes: &HashMap<String, u32>) -> Option<u32> {
		if let Some(size) = struct_sizes.get(r#type) {
			return Some(*size);
		}

		let (base, suffix) = Self::hlsl_type_base_and_suffix(r#type);
		let scalar_size = match base {
			"bool" | "float" | "int" | "uint" | "uint32_t" | "int32_t" => 4,
			"half" | "float16_t" | "uint16_t" | "int16_t" => 2,
			"double" => 8,
			_ => return None,
		};

		if suffix.is_empty() {
			return Some(scalar_size);
		}

		if let Some((rows, columns)) = suffix.split_once('x') {
			let rows = rows.parse::<u32>().ok()?;
			let columns = columns.parse::<u32>().ok()?;
			return scalar_size.checked_mul(rows)?.checked_mul(columns);
		}

		let lanes = suffix.parse::<u32>().ok()?;
		scalar_size.checked_mul(lanes)
	}

	/// Splits an HLSL scalar/vector/matrix type into its scalar base and numeric suffix.
	pub(crate) fn hlsl_type_base_and_suffix(r#type: &str) -> (&str, &str) {
		for base in ["uint32_t", "int32_t", "float16_t", "uint16_t", "int16_t"] {
			if let Some(suffix) = r#type.strip_prefix(base) {
				return (base, suffix);
			}
		}

		let split = r#type
			.find(|character: char| character.is_ascii_digit())
			.unwrap_or(r#type.len());
		(&r#type[..split], &r#type[split..])
	}

	/// Parses a fixed array count from an HLSL field name.
	pub(crate) fn hlsl_array_count(field_name: &str) -> Option<u32> {
		let open = field_name.find('[')?;
		let close = field_name[open + 1..].find(']')? + open + 1;
		field_name[open + 1..close].trim().parse().ok()
	}

	/// Parses a register declaration into a descriptor binding and set index.
	pub(crate) fn hlsl_register_binding(register: &str) -> Option<(u32, u32)> {
		let mut parts = register.split(',').map(str::trim);
		let binding = parts
			.next()
			.and_then(|register| register.strip_prefix('t').or_else(|| register.strip_prefix('u')))?
			.parse()
			.ok()?;
		let space = parts
			.next()
			.and_then(|space| space.strip_prefix("space"))
			.and_then(|space| space.parse().ok())
			.unwrap_or(0);
		Some((binding, space))
	}

	/// Advances an HLSL source index past ASCII whitespace.
	pub(crate) fn skip_hlsl_whitespace(source: &str, mut index: usize) -> usize {
		while source.as_bytes().get(index).is_some_and(u8::is_ascii_whitespace) {
			index += 1;
		}
		index
	}

	/// Finds the end of an HLSL identifier starting at the provided byte index.
	pub(crate) fn hlsl_identifier_end(source: &str, mut index: usize) -> usize {
		while source
			.as_bytes()
			.get(index)
			.is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
		{
			index += 1;
		}
		index
	}

	/// Finds the matching closing brace for an HLSL block.
	pub(crate) fn matching_hlsl_brace(source: &str, open_brace: usize) -> Option<usize> {
		let mut depth = 0u32;
		for (offset, byte) in source.as_bytes().iter().enumerate().skip(open_brace) {
			match *byte {
				b'{' => depth = depth.saturating_add(1),
				b'}' => {
					depth = depth.checked_sub(1)?;
					if depth == 0 {
						return Some(offset);
					}
				}
				_ => {}
			}
		}
		None
	}
}
