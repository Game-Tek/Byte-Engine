#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkBuffer)

enum class BufferType : unsigned char;

GS_CLASS Vk_Buffer final : public VulkanObject
{
	VkBuffer Buffer = nullptr;

	static unsigned BufferTypeToVkBufferUsageFlagBits(BufferType _BT);
public:
	Vk_Buffer(const Vk_Device& _Device, unsigned _BufferUsage, size_t _Size);
	~Vk_Buffer();

	INLINE operator VkBuffer() const { return Buffer; }
	INLINE operator const VkBuffer* () const { return &Buffer; }
};