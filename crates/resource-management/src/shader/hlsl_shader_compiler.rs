use crate::types::ShaderTypes;

/// Compiles generated HLSL into the native DXIL payload consumed by DX12.
#[cfg(target_os = "windows")]
pub(crate) fn compile_hlsl_source_to_dxil(
	source: &str,
	name: &str,
	entry_point: &str,
	stage: ShaderTypes,
) -> Result<Box<[u8]>, String> {
	use windows::Win32::Graphics::Direct3D::Dxc::{
		CLSID_DxcCompiler, DXC_CP_UTF8, DXC_OUT_OBJECT, DxcBuffer, DxcCreateInstance, IDxcBlob, IDxcCompiler3,
		IDxcIncludeHandler, IDxcResult,
	};
	use windows::core::PCWSTR;

	let target = dxil_target_profile(stage)?;
	// SAFETY: DXC owns the registered compiler class and returns a typed COM interface on success.
	let compiler = unsafe { DxcCreateInstance::<IDxcCompiler3>(&CLSID_DxcCompiler) }.map_err(|error| {
		format!(
			"Failed to create DXC while baking HLSL shader '{name}'. The most likely cause is that the DirectX Shader Compiler runtime is unavailable. Error: {error:?}"
		)
	})?;
	require_shader_model_6_9_dxc(&compiler)?;
	let source_buffer = DxcBuffer {
		Ptr: source.as_ptr().cast(),
		Size: source.len(),
		Encoding: DXC_CP_UTF8.0,
	};

	let argument_storage = vec![
		wide_argument("-E"),
		wide_argument(entry_point),
		wide_argument("-T"),
		wide_argument(target),
		wide_argument("-O3"),
		// Baked DXIL follows the same fully-bound descriptor contract as runtime DX12 compilation.
		wide_argument("-all_resources_bound"),
		// Pin the same modern HLSL and exact-width 16-bit policy used by runtime compilation.
		wide_argument("-HV"),
		wide_argument("2021"),
		wide_argument("-enable-16bit-types"),
	];
	let arguments = argument_storage
		.iter()
		.map(|argument| PCWSTR(argument.as_ptr()))
		.collect::<Vec<_>>();
	// SAFETY: The source buffer and null-terminated argument storage remain alive for the duration of the synchronous compile call.
	let result = unsafe {
		compiler.Compile::<Option<&IDxcIncludeHandler>, IDxcResult>(&source_buffer, Some(arguments.as_slice()), None)
	}
	.map_err(|error| {
		format!(
			"Failed to invoke DXC while baking HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'. Error: {error:?}"
		)
	})?;
	// SAFETY: The result is a live DXC result interface returned by the completed compile call.
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
	// SAFETY: object is a valid output slot and remains alive for the duration of this COM call.
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
	// SAFETY: The blob owns this pointer and keeps it valid until object is dropped.
	let bytecode_pointer = unsafe { object.GetBufferPointer() }.cast::<u8>();
	// SAFETY: The blob reports the exact initialized byte length for its owned buffer.
	let bytecode_size = unsafe { object.GetBufferSize() };
	// SAFETY: The pointer and size come from the same live blob allocation.
	let bytecode = unsafe { std::slice::from_raw_parts(bytecode_pointer, bytecode_size) };
	if bytecode.is_empty() {
		return Err(format!(
			"DXC returned empty DXIL output while baking HLSL shader '{name}' for entry point '{entry_point}' and target '{target}'."
		));
	}

	Ok(bytecode.to_vec().into_boxed_slice())
}

/// Verifies that the loaded compiler is the retail DXC generation used for Shader Model 6.9.
#[cfg(target_os = "windows")]
fn require_shader_model_6_9_dxc(compiler: &windows::Win32::Graphics::Direct3D::Dxc::IDxcCompiler3) -> Result<(), String> {
	use windows::Win32::Graphics::Direct3D::Dxc::IDxcVersionInfo2;
	use windows::core::Interface;

	let version_info = compiler.cast::<IDxcVersionInfo2>().map_err(|error| {
		format!(
			"Failed to query the loaded DirectX Shader Compiler identity. The most likely cause is that dxcompiler.dll predates DXC version metadata support. Error: {error:?}"
		)
	})?;
	let mut major = 0;
	let mut minor = 0;
	// SAFETY: major and minor are valid output slots and version_info remains alive for this COM call.
	unsafe { version_info.GetVersion(&mut major, &mut minor) }.map_err(|error| {
		format!(
			"Failed to read the loaded DirectX Shader Compiler version. The most likely cause is an invalid or incompatible dxcompiler.dll. Error: {error:?}"
		)
	})?;
	let mut commit_count = 0;
	let mut commit_hash = std::ptr::null_mut();
	// SAFETY: both values are valid output slots and DXC owns the returned NUL-terminated commit string.
	unsafe { version_info.GetCommitInfo(&mut commit_count, &mut commit_hash) }.map_err(|error| {
		format!(
			"Failed to read the loaded DirectX Shader Compiler commit. The most likely cause is an invalid or incompatible dxcompiler.dll. Error: {error:?}"
		)
	})?;
	if commit_hash.is_null() {
		return Err(
			"DXC returned no compiler commit identity. The most likely cause is an incomplete or unofficial dxcompiler.dll build."
				.to_string(),
		);
	}
	// SAFETY: GetCommitInfo returned a non-null NUL-terminated string owned by the live version_info object.
	let commit_hash = unsafe { std::ffi::CStr::from_ptr(commit_hash) }.to_string_lossy();
	if !dxc_version_supports_shader_model_6_9(major, minor, commit_count) {
		return Err(format!(
			"DirectX Shader Compiler {major}.{minor} commit {commit_count} ({commit_hash}) is too old. The most likely cause is that Windows loaded dxcompiler.dll from an older SDK instead of Microsoft.Direct3D.DXC 1.9.2607.13. See https://github.com/microsoft/DirectXShaderCompiler/releases/tag/v1.9.2607."
		));
	}

	Ok(())
}

/// Checks the reported DXC version against the first retail compiler with Shader Model 6.9 support.
#[cfg(any(target_os = "windows", test))]
fn dxc_version_supports_shader_model_6_9(major: u32, minor: u32, commit_count: u32) -> bool {
	const MINIMUM_MAJOR: u32 = 1;
	const MINIMUM_MINOR: u32 = 9;
	const MINIMUM_1_9_COMMIT_COUNT: u32 = 5402;

	(major, minor) > (MINIMUM_MAJOR, MINIMUM_MINOR)
		|| ((major, minor) == (MINIMUM_MAJOR, MINIMUM_MINOR) && commit_count >= MINIMUM_1_9_COMMIT_COUNT)
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

/// Selects the Shader Model 6.9 DXC profile for one generated shader.
#[cfg(any(target_os = "windows", test))]
fn dxil_target_profile(stage: ShaderTypes) -> Result<&'static str, String> {
	match stage {
		ShaderTypes::Vertex => Ok("vs_6_9"),
		ShaderTypes::Fragment => Ok("ps_6_9"),
		ShaderTypes::Compute => Ok("cs_6_9"),
		ShaderTypes::Task => Ok("as_6_9"),
		ShaderTypes::Mesh => Ok("ms_6_9"),
		_ => Err(
			"Unsupported DXIL shader stage. The most likely cause is that a standalone or material shader requested a stage outside Vertex, Fragment, Compute, Task, or Mesh."
				.to_string(),
		),
	}
}

#[cfg(target_os = "windows")]
fn wide_argument(argument: &str) -> Vec<u16> {
	argument.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn dxc_error_output(result: &windows::Win32::Graphics::Direct3D::Dxc::IDxcResult) -> String {
	use windows::Win32::Graphics::Direct3D::Dxc::{DXC_OUT_ERRORS, IDxcBlob};

	let mut errors = None;
	// SAFETY: errors is a valid output slot and result remains alive for this COM call.
	if unsafe { result.GetOutput::<IDxcBlob>(DXC_OUT_ERRORS, std::ptr::null_mut(), &mut errors) }.is_err() {
		return "DXC compilation failed and error output could not be read.".to_string();
	}

	let Some(errors) = errors else {
		return "DXC compilation failed with no error output.".to_string();
	};
	// SAFETY: The blob owns this pointer and keeps it valid until errors is dropped.
	let error_pointer = unsafe { errors.GetBufferPointer() }.cast::<u8>();
	// SAFETY: The blob reports the exact initialized byte length for its owned buffer.
	let error_size = unsafe { errors.GetBufferSize() };
	// SAFETY: The pointer and size come from the same live blob allocation.
	let bytes = unsafe { std::slice::from_raw_parts(error_pointer, error_size) };
	let message = String::from_utf8_lossy(bytes).trim().to_string();
	if message.is_empty() {
		"DXC compilation failed with empty error output.".to_string()
	} else {
		message
	}
}

#[cfg(test)]
mod tests {
	use super::{dxc_version_supports_shader_model_6_9, dxil_target_profile};
	use crate::types::ShaderTypes;

	#[test]
	fn dxil_profiles_use_shader_model_6_9_for_every_baked_stage() {
		assert_eq!(dxil_target_profile(ShaderTypes::Vertex).unwrap(), "vs_6_9");
		assert_eq!(dxil_target_profile(ShaderTypes::Fragment).unwrap(), "ps_6_9");
		assert_eq!(dxil_target_profile(ShaderTypes::Compute).unwrap(), "cs_6_9");
		assert_eq!(dxil_target_profile(ShaderTypes::Task).unwrap(), "as_6_9");
		assert_eq!(dxil_target_profile(ShaderTypes::Mesh).unwrap(), "ms_6_9");
	}

	#[test]
	fn dxil_profile_rejects_non_baked_shader_stages() {
		assert!(dxil_target_profile(ShaderTypes::RayGen).is_err());
	}

	#[test]
	fn shader_model_6_9_dxc_policy_rejects_older_compilers() {
		assert!(!dxc_version_supports_shader_model_6_9(1, 8, 9000));
		assert!(!dxc_version_supports_shader_model_6_9(1, 9, 5401));
		assert!(dxc_version_supports_shader_model_6_9(1, 9, 5402));
		assert!(dxc_version_supports_shader_model_6_9(2, 0, 1));
	}
}
