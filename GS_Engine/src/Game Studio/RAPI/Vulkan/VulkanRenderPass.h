#pragma once

#include "RAPI/RenderPass.h"

#include "RAPI/Vulkan/Vulkan.h"

class VulkanRenderPass final : public RAPI::RenderPass
{
	VkRenderPass renderPass = nullptr;

public:
	VulkanRenderPass(class VulkanRenderDevice* vulkanRenderDevice, const RAPI::RenderPassCreateInfo& renderPassDescriptor);
	~VulkanRenderPass() = default;

	void Destroy(class RenderDevice* renderDevice) override;

	VkRenderPass GetVkRenderPass() const { return renderPass; }
};
