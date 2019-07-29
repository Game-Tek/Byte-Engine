#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

#include "Extent.h"
#include "Vk_ImageView.h"

class Vk_RenderPass;
MAKE_VK_HANDLE(VkFramebuffer)

GS_CLASS Vk_Framebuffer final : public VulkanObject
{
	VkFramebuffer Framebuffer = nullptr;

public:
	Vk_Framebuffer(const Vk_Device& _Device, Extent2D _Extent, const Vk_RenderPass& _RP, const FVector<Vk_ImageView>& _Images);
	~Vk_Framebuffer();
};