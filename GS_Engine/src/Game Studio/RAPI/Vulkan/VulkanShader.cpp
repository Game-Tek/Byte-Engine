#include "Vulkan.h"

#include "VulkanShader.h"

#include "Containers/FString.h"

#include <fstream>

VulkanShader::VulkanShader(VkDevice _Device, const FString& _Name, ShaderType _ShaderType) : Shader(_ShaderType), ShaderModule(_Device, GetShaderCode(_Name))
{
}

Tuple<std::vector<char>, size_t> VulkanShader::GetShaderCode(const FString& _Name)
{
	Tuple<std::vector<char>, size_t> Result;

	std::ifstream file(_Name.c_str(), std::ios::ate | std::ios::binary);

	if (!file.is_open()) {
		throw std::runtime_error("failed to open file!");
	}

	const size_t fileSize = size_t(file.tellg());
	std::vector<char> buffer(fileSize);

	file.seekg(0);
	file.read(buffer.data(), fileSize);

	file.close();

	Result.First = buffer;
	Result.Second = fileSize;

	return Result;
}

Vk_Shader::Vk_Shader(VkDevice _Device, Tuple<std::vector<char>, size_t> _Data) : VulkanObject(_Device)
{
	VkShaderModuleCreateInfo ShaderCreateInfo = { VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
	ShaderCreateInfo.pCode = reinterpret_cast<const uint32_t*>(_Data.First.data());
	ShaderCreateInfo.codeSize = _Data.Second;

	GS_VK_CHECK(vkCreateShaderModule(_Device, &ShaderCreateInfo, ALLOCATOR, &Shader), "Failed to create Shader!")
}

Vk_Shader::~Vk_Shader()
{
	vkDestroyShaderModule(m_Device, Shader, ALLOCATOR);
}
