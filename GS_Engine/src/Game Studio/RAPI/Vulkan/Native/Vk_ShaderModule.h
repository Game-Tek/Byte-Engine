#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FString.h"

MAKE_VK_HANDLE(VkShaderModule)

struct VkShaderModuleCreateInfo;
enum VkShaderStageFlagBits;

GS_CLASS Vk_ShaderModule final : public VulkanObject
{
	VkShaderModule ShaderModule = nullptr;

	static VkShaderModuleCreateInfo CreateShaderModuleCreateInfo(const FString& _Code, VkShaderStageFlagBits _Stage);
public:
	Vk_ShaderModule(const Vk_Device& _Device, uint32* _Data, size_t _Size);
	Vk_ShaderModule(const Vk_Device& _Device, const FString& _Code, VkShaderStageFlagBits _Stage);
	~Vk_ShaderModule();

	INLINE operator VkShaderModule() const { return ShaderModule; }
};