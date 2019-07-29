#include "Vk_ShaderModule.h"

#include "RAPI/Vulkan/Vulkan.h"

Vk_ShaderModule::Vk_ShaderModule(const Vk_Device& _Device, uint32* _Data, size_t _Size) : VulkanObject(_Device)
{
	VkShaderModuleCreateInfo ShaderCreateInfo = { VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
	ShaderCreateInfo.pCode = _Data;
	ShaderCreateInfo.codeSize = _Size;

	GS_VK_CHECK(vkCreateShaderModule(m_Device, &ShaderCreateInfo, ALLOCATOR, &ShaderModule), "Failed to create Shader!")
}

Vk_ShaderModule::~Vk_ShaderModule()
{
	vkDestroyShaderModule(m_Device, ShaderModule, ALLOCATOR);
}
