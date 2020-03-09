#include "VulkanTexture.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VulkanRenderDevice.h"
#include <RAPI\Vulkan\VulkanRenderDevice.cpp>
#include <RAPI\Vulkan\VulkanCommandBuffer.h>

VulkanTexture::VulkanTexture(VulkanRenderDevice* vulkanRenderDevice, const TextureCreateInfo& textureCreateInfo) : Texture(textureCreateInfo)
{
	// CREATE STAGING BUFFER (AND DEVICE MEMORY)
	// COPY IMAGE DATA TO STAGING BUFFER
	// CREATE IMAGE (AND DEVICE MEMORY)
	// TRANSITION LAYOUT FROM UNDEFINED TO TRANSFER_DST
	// COPY STAGING BUFFER TO IMAGE
	// TRANSITION LAYOUT FROM TRANSFER_DST TO { DESIRED USE }

	VkBuffer staging_buffer = VK_NULL_HANDLE;
	VkDeviceMemory staging_buffer_memory = VK_NULL_HANDLE;

	DArray<VkFormat> formats = { FormatToVkFormat(textureCreateInfo.ImageFormat), VK_FORMAT_R8G8B8A8_UNORM };

	auto originalFormat = FormatToVkFormat(textureCreateInfo.ImageFormat);
	auto supportedFormat = vulkanRenderDevice->FindSupportedFormat(formats, VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT, VK_IMAGE_TILING_OPTIMAL);

	uint64 originalTextureSize = textureCreateInfo.ImageDataSize;
	uint64 supportedTextureSize = 0;

	if (originalFormat != supportedFormat)
	{
		switch (originalFormat)
		{
		case VK_FORMAT_R8G8B8_UNORM:
			switch (supportedFormat)
			{
			case VK_FORMAT_R8G8B8A8_UNORM:
				supportedTextureSize = (originalTextureSize / 3) * 4;
			}
		}
	}

	CreateBuffer(vulkanRenderDevice->GetVkDevice(), &staging_buffer, supportedTextureSize, VK_BUFFER_USAGE_TRANSFER_SRC_BIT, VK_SHARING_MODE_EXCLUSIVE);

	{
		VkMemoryRequirements memRequirements;
		vkGetBufferMemoryRequirements(vulkanRenderDevice->GetVkDevice(), staging_buffer, &memRequirements);

		VkMemoryAllocateInfo allocInfo{ VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
		allocInfo.allocationSize = memRequirements.size;
		allocInfo.memoryTypeIndex = vulkanRenderDevice->FindMemoryType(memRequirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);

		VK_CHECK(vkAllocateMemory(vulkanRenderDevice->GetVkDevice(), &allocInfo, vulkanRenderDevice->GetVkAllocationCallbacks(), &staging_buffer_memory));

		vkBindBufferMemory(vulkanRenderDevice->GetVkDevice(), staging_buffer, staging_buffer_memory, 0);
	}

	void* data = nullptr;
	vkMapMemory(vulkanRenderDevice->GetVkDevice(), staging_buffer_memory, 0, supportedTextureSize, 0, &data);

	if (originalFormat != supportedFormat)
	{
		switch (originalFormat)
		{
		case VK_FORMAT_R8G8B8_UNORM:
			switch (supportedFormat)
			{
			case VK_FORMAT_R8G8B8A8_UNORM:

				for (uint32 i = 0, i_n = 0; i < supportedTextureSize; i += 4, i_n += 3)
				{
					memcpy(static_cast<char*>(data) + i, static_cast<char*>(textureCreateInfo.ImageData) + i_n, 3);
					static_cast<char*>(data)[i + 3] = 0;
				}

				break;
			}
		}
	}
	else
	{
		supportedTextureSize = originalTextureSize;
		memcpy(data, textureCreateInfo.ImageData, static_cast<size_t>(supportedTextureSize));
	}

	vkUnmapMemory(vulkanRenderDevice->GetVkDevice(), staging_buffer_memory);

	VkImageCreateInfo vk_image_create_info{ VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO };
	vk_image_create_info.imageType = VK_IMAGE_TYPE_2D;
	vk_image_create_info.extent.width = textureCreateInfo.Extent.Width;
	vk_image_create_info.extent.height = textureCreateInfo.Extent.Height;
	vk_image_create_info.extent.depth = 1;
	vk_image_create_info.mipLevels = 1;
	vk_image_create_info.arrayLayers = 1;
	vk_image_create_info.format = supportedFormat;
	vk_image_create_info.tiling = VK_IMAGE_TILING_OPTIMAL;
	vk_image_create_info.initialLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
	vk_image_create_info.usage = VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_SAMPLED_BIT;
	vk_image_create_info.samples = VK_SAMPLE_COUNT_1_BIT;
	vk_image_create_info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	VK_CHECK(vkCreateImage(vulkanRenderDevice->GetVkDevice(), &vk_image_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &textureImage));

	{
		VkMemoryRequirements memRequirements;
		vkGetImageMemoryRequirements(vulkanRenderDevice->GetVkDevice(), textureImage, &memRequirements);

		VkMemoryAllocateInfo allocInfo{ VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
		allocInfo.allocationSize = memRequirements.size;
		allocInfo.memoryTypeIndex = vulkanRenderDevice->FindMemoryType(memRequirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

		VK_CHECK(vkAllocateMemory(vulkanRenderDevice->GetVkDevice(), &allocInfo, vulkanRenderDevice->GetVkAllocationCallbacks(), &textureImageMemory));

		vkBindImageMemory(vulkanRenderDevice->GetVkDevice(), textureImage, textureImageMemory, 0);
	}


	VkBufferImageCopy region{};
	region.bufferOffset = 0;
	region.bufferRowLength = 0;
	region.bufferImageHeight = 0;
	region.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	region.imageSubresource.mipLevel = 0;
	region.imageSubresource.baseArrayLayer = 0;
	region.imageSubresource.layerCount = 1;
	region.imageOffset = { 0, 0, 0 };
	region.imageExtent = { textureCreateInfo.Extent.Width, textureCreateInfo.Extent.Height, 1 };

	vkCmdCopyBufferToImage(static_cast<VulkanCommandBuffer*>(textureCreateInfo.CommandBuffer)->GetVkCommandBuffer(), staging_buffer, textureImage, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &region);

	{
		VkImageMemoryBarrier barrier = {};
		barrier.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
		barrier.oldLayout = VK_IMAGE_LAYOUT_UNDEFINED;
		barrier.newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
		barrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
		barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
		barrier.image = textureImage;
		barrier.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
		barrier.subresourceRange.baseMipLevel = 0;
		barrier.subresourceRange.levelCount = 1;
		barrier.subresourceRange.baseArrayLayer = 0;
		barrier.subresourceRange.layerCount = 1;

		VkPipelineStageFlags sourceStage;
		VkPipelineStageFlags destinationStage;

		barrier.srcAccessMask = 0;
		barrier.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;

		sourceStage = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
		destinationStage = VK_PIPELINE_STAGE_TRANSFER_BIT;

		VkBufferImageCopy region{};
		region.bufferOffset = 0;
		region.bufferRowLength = 0;
		region.bufferImageHeight = 0;
		region.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
		region.imageSubresource.mipLevel = 0;
		region.imageSubresource.baseArrayLayer = 0;
		region.imageSubresource.layerCount = 1;
		region.imageOffset = { 0, 0, 0 };
		region.imageExtent = { textureCreateInfo.Extent.Width, textureCreateInfo.Extent.Height, 1 };

		vkCmdCopyBufferToImage(static_cast<VulkanCommandBuffer*>(textureCreateInfo.CommandBuffer)->GetVkCommandBuffer(), staging_buffer, textureImage, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &region);
	}

	{
		auto to_image_layout = ImageLayoutToVkImageLayout(textureCreateInfo.Layout);

		VkImageMemoryBarrier barrier = {};
		barrier.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
		barrier.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
		barrier.newLayout = to_image_layout;
		barrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
		barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
		barrier.image = textureImage;
		barrier.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
		barrier.subresourceRange.baseMipLevel = 0;
		barrier.subresourceRange.levelCount = 1;
		barrier.subresourceRange.baseArrayLayer = 0;
		barrier.subresourceRange.layerCount = 1;

		VkPipelineStageFlags sourceStage;
		VkPipelineStageFlags destinationStage;

		if (VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL == VK_IMAGE_LAYOUT_UNDEFINED && to_image_layout == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL)
		{
			barrier.srcAccessMask = 0;
			barrier.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;

			sourceStage = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
			destinationStage = VK_PIPELINE_STAGE_TRANSFER_BIT;
		}
		else if (VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL && to_image_layout == VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL)
		{
			barrier.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
			barrier.dstAccessMask = VK_ACCESS_SHADER_READ_BIT;

			sourceStage = VK_PIPELINE_STAGE_TRANSFER_BIT;
			destinationStage = VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT;
		}
		else
		{
			throw std::invalid_argument("unsupported layout transition!");
		}

		vkCmdPipelineBarrier(static_cast<VulkanCommandBuffer*>(textureCreateInfo.CommandBuffer)->GetVkCommandBuffer(), sourceStage, destinationStage, 0, 0, nullptr, 0, nullptr, 1, &barrier);

	}

	vkDestroyBuffer(vulkanRenderDevice->GetVkDevice(), staging_buffer, vulkanRenderDevice->GetVkAllocationCallbacks());
	vkFreeMemory(vulkanRenderDevice->GetVkDevice(), staging_buffer_memory, vulkanRenderDevice->GetVkAllocationCallbacks());


	VkImageViewCreateInfo vk_image_view_create_info{ VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
	vk_image_view_create_info.image = textureImage;
	vk_image_view_create_info.viewType = VK_IMAGE_VIEW_TYPE_2D;
	vk_image_view_create_info.format = supportedFormat;
	vk_image_view_create_info.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	vk_image_view_create_info.subresourceRange.baseMipLevel = 0;
	vk_image_view_create_info.subresourceRange.levelCount = 1;
	vk_image_view_create_info.subresourceRange.baseArrayLayer = 0;
	vk_image_view_create_info.subresourceRange.layerCount = 1;

	VK_CHECK(vkCreateImageView(vulkanRenderDevice->GetVkDevice(), &vk_image_view_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &textureImageView));

	VkSamplerCreateInfo vk_sampler_create_info{ VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO };
	vk_sampler_create_info.magFilter = VK_FILTER_LINEAR;
	vk_sampler_create_info.minFilter = VK_FILTER_LINEAR;
	vk_sampler_create_info.addressModeU = VK_SAMPLER_ADDRESS_MODE_REPEAT;
	vk_sampler_create_info.addressModeV = VK_SAMPLER_ADDRESS_MODE_REPEAT;
	vk_sampler_create_info.addressModeW = VK_SAMPLER_ADDRESS_MODE_REPEAT;

	vk_sampler_create_info.anisotropyEnable = VkBool32(textureCreateInfo.Anisotropy);
	vk_sampler_create_info.maxAnisotropy = static_cast<float>(textureCreateInfo.Anisotropy == 0 ? 1 : textureCreateInfo.Anisotropy);

	vk_sampler_create_info.borderColor = VK_BORDER_COLOR_INT_OPAQUE_BLACK;
	vk_sampler_create_info.unnormalizedCoordinates = VK_FALSE;
	vk_sampler_create_info.compareEnable = VK_FALSE;
	vk_sampler_create_info.compareOp = VK_COMPARE_OP_ALWAYS;
	vk_sampler_create_info.mipmapMode = VK_SAMPLER_MIPMAP_MODE_LINEAR;
	vk_sampler_create_info.mipLodBias = 0.0f;
	vk_sampler_create_info.minLod = 0.0f;
	vk_sampler_create_info.maxLod = 0.0f;

	VK_CHECK(vkCreateSampler(vulkanRenderDevice->GetVkDevice(), &vk_sampler_create_info, vulkanRenderDevice->GetVkAllocationCallbacks(), &textureSampler));
}

void VulkanTexture::Destroy(RenderDevice* renderDevice)
{
	auto vk_render_device = static_cast<VulkanRenderDevice*>(renderDevice);
	vkDestroySampler(vk_render_device->GetVkDevice(), textureSampler, vk_render_device->GetVkAllocationCallbacks());
	vkDestroyImageView(vk_render_device->GetVkDevice(), textureImageView, vk_render_device->GetVkAllocationCallbacks());
	vkDestroyImage(vk_render_device->GetVkDevice(), textureImage, vk_render_device->GetVkAllocationCallbacks());
	vkFreeMemory(vk_render_device->GetVkDevice(), textureImageMemory, vk_render_device->GetVkAllocationCallbacks());
}
