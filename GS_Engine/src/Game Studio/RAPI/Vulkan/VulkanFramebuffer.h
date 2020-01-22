#pragma once

#include "Core.h"

#include "RAPI/Framebuffer.h"

#include "Utility/Extent.h"
#include "Native/VKFramebuffer.h"
#include "Containers/DArray.hpp"
#include "Native/VKImageView.h"
#include "Containers/FVector.hpp"
#include <RAPI/Vulkan/Vulkan.h>

class VulkanRenderDevice;
class VulkanRenderPass;
class VulkanImage;

struct VkExtent2D;

struct VkAttachmentDescription;
struct VkSubpassDescription;

class GS_API VulkanFramebuffer final : public Framebuffer
{	
	VkFramebuffer framebuffer;
	
public:
	VulkanFramebuffer(VulkanRenderDevice* _Device, const FramebufferCreateInfo& framebufferCreateInfo);
	~VulkanFramebuffer() = default;

	INLINE const VkFramebuffer& GetVkFramebuffer() const { return framebuffer; }
};