#pragma once

#include "Core.h"

#include "RAPI/Framebuffer.h"

#include "Containers/FVector.hpp"
#include <RAPI/Vulkan/Vulkan.h>

class VulkanFramebuffer final : public Framebuffer
{
	FVector<VkClearValue> clearValues;
	VkFramebuffer framebuffer = nullptr;

public:
	VulkanFramebuffer(class VulkanRenderDevice* vulkanRenderDevice, const FramebufferCreateInfo& framebufferCreateInfo);
	~VulkanFramebuffer() = default;

	void Destroy(RenderDevice* renderDevice) override;

	[[nodiscard]] VkFramebuffer GetVkFramebuffer() const { return framebuffer; }
	[[nodiscard]] const FVector<VkClearValue>& GetClearValues() const { return clearValues; }
};
