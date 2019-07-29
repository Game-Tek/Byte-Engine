#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkShaderModule)

GS_CLASS Vk_ShaderModule final : public VulkanObject
{
	VkShaderModule ShaderModule = nullptr;

public:
	Vk_ShaderModule(const Vk_Device& _Device, uint32* _Data, size_t _Size);
	~Vk_ShaderModule();

	INLINE operator VkShaderModule() const { return ShaderModule; }
};