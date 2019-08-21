#pragma once

#include "Core.h"

#include "RAPI/Framebuffer.h"

#include "Extent.h"
#include "Native/Vk_Framebuffer.h"
#include "Containers/DArray.hpp"

class VulkanRenderPass;
class VulkanImage;

struct VkExtent2D;

struct VkAttachmentDescription;
struct VkSubpassDescription;

GS_CLASS VulkanFramebuffer final : public Framebuffer
{
	Vk_Framebuffer m_Framebuffer;

	static FVector<VkImageView> ImagesToVkImageViews(const DArray<Image*>& _Images);
public:
	VulkanFramebuffer(const Vk_Device& _Device, VulkanRenderPass* _RP, Extent2D _Extent, const DArray<Image*>& _Images);
	~VulkanFramebuffer() = default;

	INLINE const Vk_Framebuffer& GetVk_Framebuffer() const { return m_Framebuffer; }
};