#include "Vulkan.h"

#include "VulkanFramebuffer.h"

#include "VulkanRenderPass.h"
#include "VulkanImage.h"
#include "Containers/FVector.hpp"

VulkanFramebuffer::VulkanFramebuffer(VkDevice _Device, RenderPass* _RP, Extent2D _Extent, Image* _Images, uint8 _ImagesCount) : Framebuffer(_Extent),
	m_Framebuffer(_Device, SCAST(VulkanRenderPass*, _RP)->GetVk_RenderPass(), Extent2DToVkExtent2D(_Extent), SCAST(VulkanImage*, _Images), _ImagesCount)
{

}

VulkanFramebuffer::~VulkanFramebuffer()
{
}