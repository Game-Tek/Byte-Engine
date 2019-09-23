#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkFence)

struct VkFenceCreateInfo;

struct GS_API VKFenceCreator final : VKObjectCreator<VkFence>
{
	VKFenceCreator(VKDevice* _Device, const VkFenceCreateInfo * _VkFCI);
};

class GS_API VKFence final : public VKObject<VkFence>
{
public:
	VKFence(const VKFenceCreator& _VKFC) : VKObject<VkFence>(_VKFC)
	{
	}

	~VKFence();

	void Wait() const;
	void Reset() const;

	static void WaitForFences(uint8 _Count, VKFence* _Fences, bool _WaitForAll);
	static void ResetFences(uint8 _Count, VKFence* _Fences);

	[[nodiscard]] bool GetStatus() const;
};