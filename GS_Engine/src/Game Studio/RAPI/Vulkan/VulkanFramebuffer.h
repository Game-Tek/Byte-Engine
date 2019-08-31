#pragma once

#include "Core.h"

#include "RAPI/Framebuffer.h"

#include "Extent.h"
#include "Native/VKFramebuffer.h"
#include "Containers/DArray.hpp"
#include "Native/VKImageView.h"
#include "Containers/FVector.hpp"

class VulkanRenderPass;
class VulkanImage;

struct VkExtent2D;

struct VkAttachmentDescription;
struct VkSubpassDescription;

GS_CLASS VulkanFramebuffer final : public Framebuffer
{
	VKFramebuffer m_Framebuffer;

	static FVector<VkImageView> ImagesToVkImageViews(const DArray<Image*>& _Images);

	static VKFramebufferCreator CreateFramebufferCreator(VKDevice* _Device, VulkanRenderPass* _RP, Extent2D _Extent, const DArray<Image*>& _Images);
public:
	VulkanFramebuffer(VKDevice* _Device, VulkanRenderPass* _RP, Extent2D _Extent, const DArray<Image*>& _Images);
	~VulkanFramebuffer() = default;

	INLINE const VKFramebuffer& GetVk_Framebuffer() const { return m_Framebuffer; }
};