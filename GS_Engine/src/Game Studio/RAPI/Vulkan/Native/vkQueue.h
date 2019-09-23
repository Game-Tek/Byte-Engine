#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

class VKFence;
MAKE_VK_HANDLE(VkQueue)
MAKE_VK_HANDLE(VkFence)
struct VkSubmitInfo;
struct VkPresentInfoKHR;

class GS_API vkQueue
{
	VkQueue Queue = nullptr;
	uint32 QueueIndex = 0;

public:
	vkQueue() = default;
	vkQueue(VkQueue _Queue, uint32 _Index);
	vkQueue(const vkQueue& _Other) = default;
	~vkQueue() = default;

	vkQueue& operator=(const vkQueue& _Other) = default;

	INLINE VkQueue& GetVkQueue() { return Queue; }
	INLINE uint32& GetQueueIndex() { return QueueIndex; }

	INLINE const VkQueue& GetVkQueue() const { return Queue; }
	INLINE const uint32& GetQueueIndex() const { return QueueIndex; }

	void Submit(const VkSubmitInfo* _SubmitInfo, VkFence _Fence) const;
	void Present(const VkPresentInfoKHR* _PresentInfo) const;
	void Wait() const;

	INLINE explicit operator VkQueue() const
	{
		return Queue;
	}

	INLINE explicit operator const VkQueue() const
	{
		return Queue;
	}
};
