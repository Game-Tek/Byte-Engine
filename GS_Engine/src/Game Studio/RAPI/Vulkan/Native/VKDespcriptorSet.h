#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkDescriptorSet)

struct VkDescriptorSetAllocateInfo;

struct GS_API VKDescriptorSetCreator : VKObjectCreator<VkDescriptorSet>
{
	VKDescriptorSetCreator(VKDevice* _Device, const VkDescriptorSetAllocateInfo* _VkDSCI);
};

class GS_API VKDescriptorSet final : public VKObject<VkDescriptorSet>
{
public:
	VKDescriptorSet(const VKDescriptorSetCreator& _Creator) : VKObject(_Creator)
	{
	}

	~VKDescriptorSet() = default;
};