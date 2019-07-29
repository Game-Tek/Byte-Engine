#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FVector.hpp"
#include "Vk_PhysicalDevice.h"

MAKE_VK_HANDLE(VkDevice)

struct VkDeviceQueueCreateInfo;

class Vk_Queue;

GS_CLASS Vk_Device
{
	VkDevice Device = nullptr;
	Vk_PhysicalDevice PhysicalDevice;

	Vk_Queue GraphicsQueue;
	Vk_Queue ComputeQueue;
	Vk_Queue TransferQueue;

	void SetVk_Queues(Vk_Queue** _Queue, const FVector<VkDeviceQueueCreateInfo>& _QCI);

	static FVector<VkDeviceQueueCreateInfo> CreateQueueInfos(QueueInfo* _QI, uint8 _QueueCount, VkPhysicalDevice _PD);
	static VkPhysicalDevice CreatePhysicalDevice(VkInstance _Instance);
	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _Type);
public:
	Vk_Device(VkInstance _Instance);
	~Vk_Device();

	uint32 FindMemoryType(uint32 _TypeFilter, uint32 _Properties) const;
	INLINE VkDevice GetVkDevice() const { return Device; }
	INLINE VkPhysicalDevice GetVkPhysicalDevice() const { return PhysicalDevice; }

	INLINE const Vk_Queue& GetGraphicsQueue() const { return GraphicsQueue; }
	INLINE const Vk_Queue& GetComputeQueue() const { return ComputeQueue; }
	INLINE const Vk_Queue& GetTransferQueue() const { return TransferQueue; }

	INLINE operator VkDevice() const { return Device; }
};
