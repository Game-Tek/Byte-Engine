#include "VulkanFramebuffer.h"

#include "Vulkan.h"

#include "VulkanRenderPass.h"

#include "RAPI/Image.h"

#include "VulkanImage.h"

FVector<VkImageView> VulkanFramebuffer::ImagesToVkImageViews(const DArray<Image*>& _Images)
{
	FVector<VkImageView> Result(_Images.length());

	for (uint8 i = 0; i < Result.capacity(); ++i)
	{
		Result.push_back(SCAST(VulkanImageBase*, _Images[i])->GetVKImageView().GetHandle());
	}

	return Result;
}

VKFramebufferCreator VulkanFramebuffer::CreateFramebufferCreator(VKDevice* _Device, VulkanRenderPass* _RP, Extent2D _Extent, const DArray<Image*>& _Images)
{
	auto t = ImagesToVkImageViews(_Images);

	VkFramebufferCreateInfo FramebufferCreateInfo = { VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
	FramebufferCreateInfo.attachmentCount = _Images.length();
	FramebufferCreateInfo.width = _Extent.Width;
	FramebufferCreateInfo.height = _Extent.Height;
	FramebufferCreateInfo.layers = 1;
	FramebufferCreateInfo.renderPass = _RP->GetVk_RenderPass().GetHandle();
	FramebufferCreateInfo.pAttachments = t.data();

	return VKFramebufferCreator(_Device, &FramebufferCreateInfo);
}

VulkanFramebuffer::VulkanFramebuffer(VKDevice* _Device, VulkanRenderPass* _RP, Extent2D _Extent, const DArray<Image*>& _Images) : Framebuffer(_Extent),
	m_Framebuffer(CreateFramebufferCreator(_Device, _RP, _Extent, _Images))
{

}
