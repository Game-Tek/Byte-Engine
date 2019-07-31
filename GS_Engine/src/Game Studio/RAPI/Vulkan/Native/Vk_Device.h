#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

#include "Vk_Queue.h"

#include "Containers/FVector.hpp"

MAKE_VK_HANDLE(VkDevice)

class Vk_PhysicalDevice;
class Vk_Instance;
struct QueueInfo;
struct VkDeviceQueueCreateInfo;

GS_CLASS Vk_Device
{
	VkDevice Device = nullptr;

	Vk_Queue GraphicsQueue;
	Vk_Queue ComputeQueue;
	Vk_Queue TransferQueue;

	void SetVk_Queues(Vk_Queue** _Queue, const FVector<VkDeviceQueueCreateInfo>& _QCI);

	static FVector<VkDeviceQueueCreateInfo> CreateQueueInfos(QueueInfo* _QI, uint8 _QueueCount, const Vk_PhysicalDevice& _PD);

public:
	Vk_Device(const Vk_Instance& _Instance, const Vk_PhysicalDevice& _PD);
	~Vk_Device();

	[[nodiscard]] uint32 FindMemoryType(uint32 _TypeFilter, uint32 _Properties) const;
	INLINE VkDevice GetVkDevice() const { return Device; }

	INLINE const Vk_Queue& GetGraphicsQueue() const { return GraphicsQueue; }
	INLINE const Vk_Queue& GetComputeQueue() const { return ComputeQueue; }
	INLINE const Vk_Queue& GetTransferQueue() const { return TransferQueue; }

	INLINE operator VkDevice() const { return Device; }
};
