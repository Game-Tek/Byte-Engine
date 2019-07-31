#include "VulkanFramebuffer.h"

#include "VulkanRenderPass.h"
#include "RAPI/Image.h"
#include "VulkanImage.h"

#include "Native/Vk_RenderPass.h"

FVector<VkImageView> ToVkImage(Image* _Images, uint8 _ImagesCount)
{
	FVector<VkImageView> Result(_ImagesCount);

	for (uint8 i = 0; i < Result.length(); ++i)
	{
		Image* f = &_Images[i];
		Result[i] = SCAST(VulkanImage*, f)->GetVkImageView();
	}

	return Result;
}

VulkanFramebuffer::VulkanFramebuffer(const Vk_Device& _Device, VulkanRenderPass* _RP, Extent2D _Extent, Image* _Images, uint8 _ImagesCount) : Framebuffer(_Extent),
	m_Framebuffer(_Device, _Extent, _RP->GetVk_RenderPass(), ToVkImage(_Images, _ImagesCount))
{

}

VulkanFramebuffer::~VulkanFramebuffer()
{
}