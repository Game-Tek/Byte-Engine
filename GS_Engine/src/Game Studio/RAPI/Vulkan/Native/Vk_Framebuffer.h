#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

#include "Extent.h"
#include "Vk_ImageView.h"
#include "Containers/FVector.hpp"

class Vk_RenderPass;
MAKE_VK_HANDLE(VkFramebuffer)

GS_CLASS Vk_Framebuffer final : public VulkanObject
{
	VkFramebuffer Framebuffer = nullptr;

public:
	Vk_Framebuffer(const Vk_Device& _Device, Extent2D _Extent, const Vk_RenderPass& _RP, const FVector<VkImageView>& _Images);
	~Vk_Framebuffer();

	INLINE operator VkFramebuffer() const { return Framebuffer; }
};