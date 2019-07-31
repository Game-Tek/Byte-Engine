#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

class Vk_Fence;
MAKE_VK_HANDLE(VkQueue)
MAKE_VK_HANDLE(VkFence)
struct VkSubmitInfo;
struct VkPresentInfoKHR;

GS_CLASS Vk_Queue
{
	VkQueue Queue = nullptr;
	uint32 QueueIndex = 0;

public:
	Vk_Queue() = default;
	Vk_Queue(VkQueue _Queue, uint32 _Index);
	Vk_Queue(const Vk_Queue& _Other) = default;
	~Vk_Queue() = default;

	Vk_Queue& operator=(const Vk_Queue& _Other) = default;

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
