#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkCommandBuffer)

struct VkCommandBufferAllocateInfo;

struct VKCommandBufferCreator : VKObjectCreator<VkCommandBuffer>
{
	VKCommandBufferCreator(VKDevice* _Device, const VkCommandBufferAllocateInfo* _VkCBCI);
};

class VKCommandPool;
struct VkCommandBufferBeginInfo;

class VKCommandBuffer final : public VKObject<VkCommandBuffer>
{
public:
	explicit VKCommandBuffer(const VKCommandBufferCreator& _VKCBC) : VKObject<VkCommandBuffer>(_VKCBC)
	{
	}

	~VKCommandBuffer() = default;

	void Free(const VKCommandPool& _CP) const;
	void Reset() const;
	void Begin(VkCommandBufferBeginInfo* _CBBI);
	void End();
};
