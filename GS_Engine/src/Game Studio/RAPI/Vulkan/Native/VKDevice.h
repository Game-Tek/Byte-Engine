#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

#include "vkQueue.h"

#include "Containers/FVector.hpp"

MAKE_VK_HANDLE(VkDevice)

class vkPhysicalDevice;
class VKInstance;
struct QueueInfo;
struct VkDeviceQueueCreateInfo;

GS_CLASS VKDevice
{
	VkDevice Device = nullptr;

	vkQueue GraphicsQueue;
	vkQueue ComputeQueue;
	vkQueue TransferQueue;

	void SetVk_Queues(vkQueue** _Queue, const FVector<VkDeviceQueueCreateInfo>& _QCI);

	static FVector<VkDeviceQueueCreateInfo> CreateQueueInfos(QueueInfo* _QI, uint8 _QueueCount, const vkPhysicalDevice& _PD);

public:
	VKDevice(const VKInstance& _Instance, const vkPhysicalDevice& _PD);
	~VKDevice();

	[[nodiscard]] uint32 FindMemoryType(uint32 _TypeFilter, uint32 _Properties) const;
	INLINE VkDevice GetVkDevice() const { return Device; }

	INLINE const vkQueue& GetGraphicsQueue() const { return GraphicsQueue; }
	INLINE const vkQueue& GetComputeQueue() const { return ComputeQueue; }
	INLINE const vkQueue& GetTransferQueue() const { return TransferQueue; }

	INLINE operator VkDevice() const { return Device; }
};
