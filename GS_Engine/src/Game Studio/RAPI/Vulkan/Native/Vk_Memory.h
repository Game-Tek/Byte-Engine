#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

class Vk_Image;
MAKE_VK_HANDLE(VkDeviceMemory)

struct VkMemoryRequirements;

class Vk_Buffer;
class Vk_CommandPool;
class Vk_Queue;

GS_CLASS Vk_Memory final : public VulkanObject
{
	VkDeviceMemory Memory = nullptr;

public:
	Vk_Memory(const Vk_Device& _Device);
	~Vk_Memory();
	void AllocateDeviceMemory(const VkMemoryRequirements& _MR, unsigned _MemProps);
	void* CopyToMappedMemory(void* _Data, size_t _Size);
	void CopyToDevice(const Vk_Buffer& _SrcBuffer, const Vk_Buffer& _DstBuffer, const Vk_CommandPool& _CP, const Vk_Queue& _Queue, size_t _Size) const;

	void BindBufferMemory(const Vk_Buffer& _Buffer) const;
	void BindImageMemory(const Vk_Image& _Image) const;

	INLINE operator VkDeviceMemory() const { return Memory; }
};