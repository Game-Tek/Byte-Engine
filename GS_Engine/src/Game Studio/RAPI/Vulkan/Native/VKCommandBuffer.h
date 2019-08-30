#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkCommandBuffer)

struct VkCommandBufferAllocateInfo;

GS_STRUCT VKCommandBufferCreator : VKObjectCreator<VkCommandBuffer>
{
	VKCommandBufferCreator(const VKDevice & _Device, const VkCommandBufferAllocateInfo * _VkCBCI);
};

class VKCommandPool;
struct VkCommandBufferBeginInfo;

GS_CLASS VKCommandBuffer final : public VKObject<VkCommandBuffer>
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