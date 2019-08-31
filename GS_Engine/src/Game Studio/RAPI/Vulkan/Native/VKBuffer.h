#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkBuffer)

struct VkBufferCreateInfo;

GS_STRUCT VKBufferCreator : VKObjectCreator<VkBuffer>
{
	VKBufferCreator(VKDevice* _Device, const VkBufferCreateInfo * _VkBCI);
};

struct VkMemoryRequirements;

enum class BufferType : unsigned char;

GS_CLASS VKBuffer final : public VKObject<VkBuffer>
{
	static unsigned BufferTypeToVkBufferUsageFlagBits(BufferType _BT);
public:
	VKBuffer(const VKBufferCreator& _VKBC) : VKObject<VkBuffer>(_VKBC)
	{
	}

	~VKBuffer();

	[[nodiscard]] VkMemoryRequirements GetMemoryRequirements() const;
};