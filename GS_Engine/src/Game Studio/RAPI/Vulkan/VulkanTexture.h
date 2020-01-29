#pragma once

#include "RAPI/Texture.h"

#include "RAPI/Vulkan/VulkanBase.h"
#include "Native/VKImage.h"

MAKE_VK_HANDLE(VkBuffer)
MAKE_VK_HANDLE(VkDeviceMemory)
MAKE_VK_HANDLE(VkImageView)
MAKE_VK_HANDLE(VkSampler)

struct VulkanTextureCreateInfo
{
	VkImage TextureImage = VK_NULL_HANDLE;
	VkDeviceMemory TextureImageMemory = VK_NULL_HANDLE;
	VkImageView TextureImageView = VK_NULL_HANDLE;
	VkSampler TextureSampler = VK_NULL_HANDLE;
};

class VulkanTexture : public Texture
{
	VkImage textureImage = VK_NULL_HANDLE;
	VkDeviceMemory textureImageMemory = VK_NULL_HANDLE;
	VkImageView textureImageView = VK_NULL_HANDLE;
	VkSampler textureSampler = VK_NULL_HANDLE;

public:
	explicit VulkanTexture(const TextureCreateInfo& textureCreateInfo, const VulkanTextureCreateInfo& VTCI_) :
		Texture(textureCreateInfo), textureImage(VTCI_.TextureImage), textureImageMemory(VTCI_.TextureImageMemory),
		textureImageView(VTCI_.TextureImageView), textureSampler(VTCI_.TextureSampler)
	{
	}

	[[nodiscard]] VkImageView GetImageView() const { return textureImageView; }
	[[nodiscard]] VkSampler GetImageSampler() const { return textureSampler; }
};
