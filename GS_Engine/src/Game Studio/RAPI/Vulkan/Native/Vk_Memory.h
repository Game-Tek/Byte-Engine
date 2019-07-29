#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkDeviceMemory)

struct VkMemoryRequirements;

class Vk_Buffer;
class Vk_CommandPool;
class Vk_Queue;

GS_CLASS Vk_Memory final : public VulkanObject
{
	VkDeviceMemory Memory = nullptr;

public:
	Vk_Memory(const Vk_Device& _Device, const Vk_Buffer& _Buffer);
	~Vk_Memory();
	void AllocateDeviceMemory(VkMemoryRequirements* _MR);
	void* CopyToMappedMemory(void* _Data, size_t _Size);
	void CopyToDevice(const Vk_Buffer& _SrcBuffer, const Vk_Buffer& _DstBuffer, const Vk_CommandPool& _CP, const Vk_Queue& _Queue, size_t _Size);

	INLINE operator VkDeviceMemory() const { return Memory; }
};