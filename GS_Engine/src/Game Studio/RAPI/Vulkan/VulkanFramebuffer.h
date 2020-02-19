#pragma once

#include "Core.h"

#include "RAPI/Framebuffer.h"

#include "Native/VKFramebuffer.h"
#include "Containers/FVector.hpp"
#include <RAPI/Vulkan/Vulkan.h>

class VulkanRenderDevice;
class VulkanRenderPass;
class VulkanRenderTarget;

struct VkExtent2D;

struct VkAttachmentDescription;
struct VkSubpassDescription;

class VulkanFramebuffer final : public Framebuffer
{
	FVector<VkClearValue> clearValues;
	VkFramebuffer framebuffer;

public:
	VulkanFramebuffer(VulkanRenderDevice* _Device, const FramebufferCreateInfo& framebufferCreateInfo);
	~VulkanFramebuffer() = default;

	INLINE const VkFramebuffer& GetVkFramebuffer() const { return framebuffer; }
	[[nodiscard]] const FVector<VkClearValue>& GetClearValues() const { return clearValues; }
};
