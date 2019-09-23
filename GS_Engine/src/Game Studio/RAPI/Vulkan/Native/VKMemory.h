#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

class VKImage;
MAKE_VK_HANDLE(VkDeviceMemory)

struct VkMemoryRequirements;

class VKBuffer;
class VKCommandPool;
class vkQueue;

struct VkMemoryAllocateInfo;

struct GS_API VKMemoryCreator final : VKObjectCreator<VkDeviceMemory>
{
	VKMemoryCreator(VKDevice* _Device, const VkMemoryAllocateInfo * _VkMAI);
};

class GS_API VKMemory final : public VKObject<VkDeviceMemory>
{
public:
	VKMemory(const VKMemoryCreator& _VKMC) : VKObject<VkDeviceMemory>(_VKMC)
	{
	}

	~VKMemory();

	void* SingleCopyToMappedMemory(void* _Data, size_t _Size) const;

	[[nodiscard]] void* MapMemory(size_t _Offset, size_t _Size) const;
	void CopyToMappedMemory(void* _Src, void* _Dst, size_t _Size) const;
	void UnmapMemory() const;

	void CopyToDevice(const VKBuffer& _SrcBuffer, const VKBuffer& _DstBuffer, const VKCommandPool& _CP, const vkQueue& _Queue, size_t _Size) const;

	void BindBufferMemory(const VKBuffer& _Buffer) const;
	void BindImageMemory(const VKImage& _Image) const;
};