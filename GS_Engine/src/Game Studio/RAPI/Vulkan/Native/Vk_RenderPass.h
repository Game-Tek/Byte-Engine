#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

#include "Containers/Tuple.h"
#include "Containers/FVector.hpp"

MAKE_VK_HANDLE(VkRenderPass)

struct VkAttachmentDescription;
struct VkSubpassDescription;

GS_CLASS Vk_RenderPass final : public VulkanObject
{
	VkRenderPass RenderPass = nullptr;

public:
	Vk_RenderPass(const Vk_Device& _Device, const Tuple<FVector<VkAttachmentDescription>, FVector<VkSubpassDescription>>& _Info);
	~Vk_RenderPass();

	INLINE operator VkRenderPass() const { return RenderPass; }
};