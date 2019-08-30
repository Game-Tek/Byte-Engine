#include "VKShaderModule.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

#include <iostream>
#include <string>

#include <vulkan/shaderc/shaderc.hpp>
#include "Logger.h"

VKShaderModuleCreator::VKShaderModuleCreator(const VKDevice& _Device, const VkShaderModuleCreateInfo* _VkSMCI) : VKObjectCreator<VkShaderModule>(_Device)
{
	GS_VK_CHECK(vkCreateShaderModule(m_Device, _VkSMCI, ALLOCATOR, &Handle), "Failed to create Shader!")
}

VKShaderModule::~VKShaderModule()
{
	vkDestroyShaderModule(m_Device, Handle, ALLOCATOR);
}

DArray<uint32> VKShaderModule::CompileGLSLToSpirV(const FString& _Code, const FString& _ShaderName, unsigned _SSFB)
{
	shaderc_shader_kind Stage;

	switch (_SSFB)
	{
	case VK_SHADER_STAGE_VERTEX_BIT:					Stage = shaderc_vertex_shader;			break;
	case VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT:		Stage = shaderc_tess_control_shader;	break;
	case VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT:	Stage = shaderc_tess_evaluation_shader;	break;
	case VK_SHADER_STAGE_GEOMETRY_BIT:					Stage = shaderc_geometry_shader;		break;
	case VK_SHADER_STAGE_FRAGMENT_BIT:					Stage = shaderc_fragment_shader;		break;
	case VK_SHADER_STAGE_COMPUTE_BIT:					Stage = shaderc_compute_shader;			break;
	default:											Stage = shaderc_spirv_assembly;			break;
	}

	const shaderc::Compiler SpirCompiler;
	shaderc::CompileOptions SpirOptions;
	SpirOptions.SetTargetSpirv(shaderc_spirv_version_1_1);
	SpirOptions.SetTargetEnvironment(shaderc_target_env_vulkan, shaderc_env_version_vulkan_1_1);
	SpirOptions.SetSourceLanguage(shaderc_source_language_glsl);
	SpirOptions.SetOptimizationLevel(shaderc_optimization_level_performance);
	const auto Module = SpirCompiler.CompileGlslToSpv(_Code.c_str(), Stage, _ShaderName.c_str(), SpirOptions);

	if(Module.GetCompilationStatus() != shaderc_compilation_status_success)
	{
		GS_BASIC_LOG_ERROR("Failed to compile shader: %s. Errors: %s", _ShaderName.c_str(), Module.GetErrorMessage().c_str())
	}

	return DArray<uint32>(Module.cbegin(), Module.cend());
}
