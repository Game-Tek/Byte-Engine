#pragma once

#include "RAPI/RenderPass.h"

#include "RAPI/Vulkan/Vulkan.h"

class VulkanRenderPass final : public RAPI::RenderPass
{
	VkRenderPass renderPass = nullptr;

public:
	VulkanRenderPass(class VulkanRenderDevice* vulkanRenderDevice, const RAPI::RenderPassDescriptor& renderPassDescriptor);
	~VulkanRenderPass() = default;

	VkRenderPass GetVkRenderPass() const { return renderPass; }
};
