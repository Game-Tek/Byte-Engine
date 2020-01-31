#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

namespace RAPI {
	enum class BufferType : unsigned char;
}

MAKE_VK_HANDLE(VkBuffer)

struct VkBufferCreateInfo;

struct GS_API VKBufferCreator : VKObjectCreator<VkBuffer>
{
	VKBufferCreator(VKDevice* _Device, const VkBufferCreateInfo* _VkBCI);
};

struct VkMemoryRequirements;

enum class BufferType : unsigned char;

class VKBuffer final : public VKObject<VkBuffer>
{
	static unsigned BufferTypeToVkBufferUsageFlagBits(RAPI::BufferType _BT);
public:
	VKBuffer(const VKBufferCreator& _VKBC) : VKObject<VkBuffer>(_VKBC)
	{
	}

	~VKBuffer();

	[[nodiscard]] VkMemoryRequirements GetMemoryRequirements() const;
};
