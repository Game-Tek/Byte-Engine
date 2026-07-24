use crate::types::ShaderTypes;

/// Compiles generated HLSL into the native DXIL payload consumed by DX12.
#[cfg(target_os = "windows")]
pub(crate) fn compile_hlsl_source_to_dxil(
	source: &str,
	name: &str,
	entry_point: &str,
	stage: ShaderTypes,
) -> Result<Box<[u8]>, String> {
	use windows::core::PCWSTR;
	use windows::Win32::Graphics::Direct3D::Dxc::{
		CLSID_DxcCompiler, DxcBuffer, DxcCreateInstance, IDxcBlob, IDxcCompiler3, IDxcIncludeHandler, IDxcResult, DXC_CP_UTF8,
		DXC_OUT_OBJECT,
	};

	let target = dxil_target_profile(stage, source)?;
	let compiler = unsafe { DxcCreateInstance::<IDxcCompiler3>(&CLSID_DxcCompiler) }.map_err(|error| {
		format!(
			"Failed to create DXC while baking HLSL shader '{name}'. The most likely cause is that the DirectX Shader Compiler runtime is unavailable. Error: {error:?}"
		)
	})?;
	let source_buffer = DxcBuffer {
		Ptr: source.as_ptr().cast(),
		Size: source.len(),
		Encoding: DXC_CP_UTF8.0,
	};

	let mut argument_storage = vec![
		wide_argument("-E"),
		wide_argument(entry_point),
		wide_argument("-T"),
		wide_argument(target),
		wide_argument("-O3"),
	];
	if hlsl_uses_native_16_bit_types(source) {
		argument_storage.push(wide_argument("-enable-16bit-types"));
	}
	let arguments = argument_storage
		.iter()
		.map(|argument| PCWSTR(argument.as_ptr()))
		.collect::<Vec<_>>();
	let result = unsafe {
		compiler.Compile::<Option<&IDxcIncludeHandler>, IDxcResult>(&source_buffer, Some(arguments.as_slice()), None)
	}
	.map_err(|error| {
		format!(
			"Failed to invoke DXC while baking HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'. Error: {error:?}"
		)
	})?;
	let status = unsafe { result.GetStatus() }.map_err(|error| {
		format!(
			"Failed to read DXC status while baking HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'. Error: {error:?}"
		)
	})?;
	if status.is_err() {
		return Err(format!(
			"Failed to compile HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'. DXC reported: {}",
			dxc_error_output(&result)
		));
	}

	let mut object = None;
	unsafe { result.GetOutput::<IDxcBlob>(DXC_OUT_OBJECT, std::ptr::null_mut(), &mut object) }.map_err(|error| {
		format!(
			"Failed to read DXIL output while baking HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'. Error: {error:?}"
		)
	})?;
	let object = object.ok_or_else(|| {
		format!(
			"DXC returned no DXIL output while baking HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'."
		)
	})?;
	let bytecode = unsafe { std::slice::from_raw_parts(object.GetBufferPointer().cast::<u8>(), object.GetBufferSize()) };
	if bytecode.is_empty() {
		return Err(format!(
			"DXC returned empty DXIL output while baking HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'."
		));
	}

	Ok(bytecode.to_vec().into_boxed_slice())
}

/// Reports the unsupported host explicitly when tooling calls the compiler outside Windows.
#[cfg(not(target_os = "windows"))]
pub(crate) fn compile_hlsl_source_to_dxil(
	_source: &str,
	_name: &str,
	_entry_point: &str,
	_stage: ShaderTypes,
) -> Result<Box<[u8]>, String> {
	Err(
		"DXIL compilation is only supported on Windows. The most likely cause is that a Windows shader artifact was requested from a non-Windows bake host."
			.to_string(),
	)
}

/// Selects the minimum DXC shader-model profile needed by one generated shader.
fn dxil_target_profile(stage: ShaderTypes, source: &str) -> Result<&'static str, String> {
	let native_16_bit_types = hlsl_uses_native_16_bit_types(source);
	match (stage, native_16_bit_types) {
		(ShaderTypes::Vertex, false) => Ok("vs_6_0"),
		(ShaderTypes::Vertex, true) => Ok("vs_6_2"),
		(ShaderTypes::Fragment, false) => Ok("ps_6_0"),
		(ShaderTypes::Fragment, true) => Ok("ps_6_2"),
		(ShaderTypes::Compute, false) => Ok("cs_6_0"),
		(ShaderTypes::Compute, true) => Ok("cs_6_2"),
		_ => Err(
			"Unsupported DXIL shader stage. The most likely cause is that a standalone or material shader requested a stage outside Vertex, Fragment, or Compute."
				.to_string(),
		),
	}
}

fn hlsl_uses_native_16_bit_types(source: &str) -> bool {
	source
		.split(|character: char| character != '_' && !character.is_ascii_alphanumeric())
		.any(|token| {
			["uint16_t", "int16_t", "float16_t"].iter().any(|&native_type| {
				let Some(suffix) = token.strip_prefix(native_type) else {
					return false;
				};

				matches!(suffix.as_bytes(), [] | [b'1'..=b'4'] | [b'1'..=b'4', b'x', b'1'..=b'4'])
			})
		})
}

#[cfg(target_os = "windows")]
fn wide_argument(argument: &str) -> Vec<u16> {
	argument.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn dxc_error_output(result: &windows::Win32::Graphics::Direct3D::Dxc::IDxcResult) -> String {
	use windows::Win32::Graphics::Direct3D::Dxc::{IDxcBlob, DXC_OUT_ERRORS};

	let mut errors = None;
	if unsafe { result.GetOutput::<IDxcBlob>(DXC_OUT_ERRORS, std::ptr::null_mut(), &mut errors) }.is_err() {
		return "DXC compilation failed and error output could not be read.".to_string();
	}

	let Some(errors) = errors else {
		return "DXC compilation failed with no error output.".to_string();
	};
	let bytes = unsafe { std::slice::from_raw_parts(errors.GetBufferPointer().cast::<u8>(), errors.GetBufferSize()) };
	let message = String::from_utf8_lossy(bytes).trim().to_string();
	if message.is_empty() {
		"DXC compilation failed with empty error output.".to_string()
	} else {
		message
	}
}

#[cfg(test)]
mod tests {
	use super::dxil_target_profile;
	use crate::types::ShaderTypes;

	#[test]
	fn dxil_profiles_cover_baked_stages_and_upgrade_native_16_bit_source() {
		assert_eq!(dxil_target_profile(ShaderTypes::Vertex, "float4 value;").unwrap(), "vs_6_0");
		assert_eq!(dxil_target_profile(ShaderTypes::Fragment, "float4 value;").unwrap(), "ps_6_0");
		assert_eq!(dxil_target_profile(ShaderTypes::Compute, "float4 value;").unwrap(), "cs_6_0");
		assert_eq!(
			dxil_target_profile(ShaderTypes::Compute, "RWStructuredBuffer<uint16_t4> values;").unwrap(),
			"cs_6_2"
		);
	}

	#[test]
	fn dxil_profile_rejects_non_baked_shader_stages() {
		assert!(dxil_target_profile(ShaderTypes::Mesh, "float4 value;").is_err());
	}
}
