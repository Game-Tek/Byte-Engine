#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkFence)

GS_CLASS Vk_Fence final : public VulkanObject
{
	VkFence Fence = nullptr;

public:
	Vk_Fence(const Vk_Device& _Device, bool _InitSignaled);
	~Vk_Fence();

	static void WaitForFences(uint8 _Count, Vk_Fence* _Fences, bool _WaitForAll);
	static void ResetFences(uint8 _Count, Vk_Fence* _Fences);

	INLINE operator VkFence() const { return Fence; }

	[[nodiscard]] bool GetStatus() const;
};