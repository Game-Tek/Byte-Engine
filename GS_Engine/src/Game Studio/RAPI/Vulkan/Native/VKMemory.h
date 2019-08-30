#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

class VKImage;
MAKE_VK_HANDLE(VkDeviceMemory)

struct VkMemoryRequirements;

class VKBuffer;
class VKCommandPool;
class vkQueue;

struct VkMemoryAllocateInfo;

GS_STRUCT VKMemoryCreator final : VKObjectCreator<VkDeviceMemory>
{
	VKMemoryCreator(const VKDevice & _Device, const VkMemoryAllocateInfo * _VkMAI);
};

GS_CLASS VKMemory final : public VKObject<VkDeviceMemory>
{
public:
	VKMemory(const VKMemoryCreator& _VKMC) : VKObject<VkDeviceMemory>(_VKMC)
	{
	}

	~VKMemory();

	void* CopyToMappedMemory(void* _Data, size_t _Size) const;
	void CopyToDevice(const VKBuffer& _SrcBuffer, const VKBuffer& _DstBuffer, const VKCommandPool& _CP, const vkQueue& _Queue, size_t _Size) const;

	void BindBufferMemory(const VKBuffer& _Buffer) const;
	void BindImageMemory(const VKImage& _Image) const;
};