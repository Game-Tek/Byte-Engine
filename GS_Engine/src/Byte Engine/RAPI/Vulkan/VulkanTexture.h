#pragma once

#include "RAPI/Texture.h"

#include "Vulkan.h"

class VulkanTexture : public Texture
{
	VkImage textureImage = nullptr;
	VkDeviceMemory textureImageMemory = nullptr;
	VkImageView textureImageView = nullptr;
	VkSampler textureSampler = nullptr;

public:
	VulkanTexture(class VulkanRenderDevice* vulkanRenderDevice, const TextureCreateInfo& textureCreateInfo);

	void Destroy(class RenderDevice* renderDevice) override;

	[[nodiscard]] VkImageView GetImageView() const { return textureImageView; }
	[[nodiscard]] VkSampler GetImageSampler() const { return textureSampler; }
};
