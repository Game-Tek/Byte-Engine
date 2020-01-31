#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkDescriptorSet)

struct VkDescriptorSetAllocateInfo;

struct VKDescriptorSetCreator : VKObjectCreator<VkDescriptorSet>
{
	VKDescriptorSetCreator(VKDevice* _Device, const VkDescriptorSetAllocateInfo* _VkDSCI);
};

class VKDescriptorSet final : public VKObject<VkDescriptorSet>
{
public:
	VKDescriptorSet(const VKDescriptorSetCreator& _Creator) : VKObject(_Creator)
	{
	}

	~VKDescriptorSet() = default;
};
