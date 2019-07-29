#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkRenderPass)

struct VkAttachmentDescription;
struct VkSubpassDescription;

GS_CLASS Vk_RenderPass final : public VulkanObject
{
	VkRenderPass RenderPass = nullptr;

public:
	Vk_RenderPass(const Vk_Device& _Device, const FVector<VkAttachmentDescription>& _Attachments, const FVector<VkSubpassDescription>& _Subpasses);
	~Vk_RenderPass();

	INLINE operator VkRenderPass() const { return RenderPass; }
};