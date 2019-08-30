#include "VulkanFramebuffer.h"

#include "VulkanRenderPass.h"
#include "RAPI/Image.h"

#include "VulkanImage.h"

#include "Native/VKRenderPass.h"

FVector<VkImageView> VulkanFramebuffer::ImagesToVkImageViews(const DArray<Image*>& _Images)
{
	FVector<VkImageView> Result(_Images.length());

	for (uint8 i = 0; i < Result.capacity(); ++i)
	{
		Result.push_back(SCAST(VulkanImageBase*, _Images[i])->GetVk_ImageView());
	}

	return Result;
}

VulkanFramebuffer::VulkanFramebuffer(const VKDevice& _Device, VulkanRenderPass* _RP, Extent2D _Extent, const DArray<Image*>& _Images) : Framebuffer(_Extent),
	m_Framebuffer(_Device, _Extent, _RP->GetVk_RenderPass(), ImagesToVkImageViews(_Images))
{

}
