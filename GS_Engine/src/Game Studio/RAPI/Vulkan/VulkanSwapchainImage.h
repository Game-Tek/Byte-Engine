#pragma once

#include "Core.h"

#include "RAPI/RenderTarget.h"

#include "VulkanRenderTarget.h"

class VulkanSwapchainImage final : public VulkanRenderTargetBase
{
public:
	VulkanSwapchainImage(VulkanRenderDevice* device, const RenderTargetCreateInfo& imageCreateInfo, VkImage image);
	~VulkanSwapchainImage() = default;
};
