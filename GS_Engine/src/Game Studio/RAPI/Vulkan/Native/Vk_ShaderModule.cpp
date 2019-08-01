#include "Vk_ShaderModule.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

#include <vulkan/glslang/Public/ShaderLang.h>
#include <vulkan/SPIRV/GlslangToSpv.h>

VkShaderModuleCreateInfo Vk_ShaderModule::CreateShaderModuleCreateInfo(const FString& _Code, VkShaderStageFlagBits _Stage)
{
	EShLanguage ShaderType;

	switch (_Stage)
	{
	case VK_SHADER_STAGE_VERTEX_BIT:					ShaderType = EShLangVertex;				break;
	case VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT:		ShaderType = EShLangTessControl;		break;
	case VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT:	ShaderType = EShLangTessEvaluation;		break;
	case VK_SHADER_STAGE_GEOMETRY_BIT:					ShaderType = EShLangGeometry;			break;
	case VK_SHADER_STAGE_FRAGMENT_BIT:					ShaderType = EShLangFragment;			break;
	case VK_SHADER_STAGE_COMPUTE_BIT:					ShaderType = EShLangCompute;			break;
	default:											ShaderType = EShLangCount;				break;
	}


	glslang::InitializeProcess();

	glslang::TShader Shader(ShaderType);

	const char* String = _Code.c_str();
	Shader.setStrings(&String, 1);

	const auto ClientInputSemanticsVersion = 100; // maps to, say, #define VULKAN 100
	const glslang::EShTargetClientVersion VulkanClientVersion = glslang::EShTargetVulkan_1_1;
	const glslang::EShTargetLanguageVersion TargetVersion = glslang::EShTargetSpv_1_1;

	Shader.setEnvInput(glslang::EShSourceGlsl, ShaderType, glslang::EShClientVulkan, ClientInputSemanticsVersion);
	Shader.setEnvClient(glslang::EShClientVulkan, VulkanClientVersion);
	Shader.setEnvTarget(glslang::EShTargetSpv, TargetVersion);

	const TBuiltInResource DefaultTBuiltInResource = { };
	TBuiltInResource Resources;
	Resources = DefaultTBuiltInResource;
	EShMessages messages = (EShMessages)(EShMsgSpvRules | EShMsgVulkanRules);

	Shader.parse(&Resources, ClientInputSemanticsVersion, false, messages);

	glslang::TProgram Program;
	Program.addShader(&Shader);

	std::vector<unsigned int> SpirV;
	spv::SpvBuildLogger logger;
	glslang::SpvOptions spvOptions;
	glslang::GlslangToSpv(*Program.getIntermediate(ShaderType), SpirV, &logger, &spvOptions);

	VkShaderModuleCreateInfo ShaderCreateInfo = { VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
	ShaderCreateInfo.pCode = SpirV.data();
	ShaderCreateInfo.codeSize = (SpirV.size() * sizeof(uint32)) / 4;
}

Vk_ShaderModule::Vk_ShaderModule(const Vk_Device& _Device, uint32* _Data, size_t _Size) : VulkanObject(_Device)
{
	VkShaderModuleCreateInfo ShaderCreateInfo = { VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
	ShaderCreateInfo.pCode = _Data;
	ShaderCreateInfo.codeSize = _Size;

	GS_VK_CHECK(vkCreateShaderModule(m_Device, &ShaderCreateInfo, ALLOCATOR, &ShaderModule), "Failed to create Shader!")
}

Vk_ShaderModule::Vk_ShaderModule(const Vk_Device& _Device, const FString& _Code, VkShaderStageFlagBits _Stage) : VulkanObject(_Device)
{
	VkShaderModuleCreateInfo ShaderCreateInfo = CreateShaderModuleCreateInfo(_Code, _Stage);

	GS_VK_CHECK(vkCreateShaderModule(m_Device, &ShaderCreateInfo, ALLOCATOR, &ShaderModule), "Failed to create Shader!")
}

Vk_ShaderModule::~Vk_ShaderModule()
{
	vkDestroyShaderModule(m_Device, ShaderModule, ALLOCATOR);
}
