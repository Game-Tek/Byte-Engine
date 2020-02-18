#pragma once

#include "RAPI/Texture.h"

#include "Vulkan.h"

struct VulkanTextureCreateInfo
{
	VkImage TextureImage = nullptr;
	VkDeviceMemory TextureImageMemory = nullptr;
	VkImageView TextureImageView = nullptr;
	VkSampler TextureSampler = nullptr;
};

class VulkanTexture : public Texture
{
	VkImage textureImage = nullptr;
	VkDeviceMemory textureImageMemory = nullptr;
	VkImageView textureImageView = nullptr;
	VkSampler textureSampler = nullptr;

public:
	explicit VulkanTexture(const TextureCreateInfo& textureCreateInfo, const VulkanTextureCreateInfo& VTCI_) :
		Texture(textureCreateInfo), textureImage(VTCI_.TextureImage), textureImageMemory(VTCI_.TextureImageMemory),
		textureImageView(VTCI_.TextureImageView), textureSampler(VTCI_.TextureSampler)
	{
	}

	[[nodiscard]] VkImageView GetImageView() const { return textureImageView; }
	[[nodiscard]] VkSampler GetImageSampler() const { return textureSampler; }
};
