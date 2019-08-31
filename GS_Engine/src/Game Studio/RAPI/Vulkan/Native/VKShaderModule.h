#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FString.h"
#include "Containers/DArray.hpp"

#include <vector>

MAKE_VK_HANDLE(VkShaderModule)

struct VkShaderModuleCreateInfo;

GS_STRUCT VKShaderModuleCreator final : public VKObjectCreator<VkShaderModule>
{
	VKShaderModuleCreator(VKDevice* _Device, const VkShaderModuleCreateInfo * _VkSMCI);
};

GS_CLASS VKShaderModule final : public VKObject<VkShaderModule>
{
public:
	VKShaderModule(const VKShaderModuleCreator& _VKSMC) : VKObject<VkShaderModule>(_VKSMC)
	{
	}

	~VKShaderModule();

	static DArray<uint32, uint32> CompileGLSLToSpirV(const FString& _Code, const FString& _ShaderName, unsigned _SSFB);
};