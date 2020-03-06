#include "Vulkan.h"

#include "VulkanRenderDevice.h"

#include "VulkanRenderContext.h"
#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"
#include "VulkanMesh.h"
#include "VulkanRenderTarget.h"
#include "VulkanUniformBuffer.h"
#include "VulkanTexture.h"

VKCommandPoolCreator VulkanRenderDevice::CreateCommandPool()
{
	VkCommandPoolCreateInfo CommandPoolCreateInfo = {VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
	CommandPoolCreateInfo.flags = VK_COMMAND_POOL_CREATE_TRANSIENT_BIT;

	return VKCommandPoolCreator(&Device, &CommandPoolCreateInfo);
}

void AllocateCommandBuffer(VkDevice* device_, VkCommandPool* command_pool_,
                           VkCommandBuffer* command_buffer_, VkCommandBufferLevel command_buffer_level_,
                           uint8 command_buffer_count_)
{
	VkCommandBufferAllocateInfo allocInfo = {};
	allocInfo.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO;
	allocInfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
	allocInfo.commandPool = *command_pool_;
	allocInfo.commandBufferCount = command_buffer_count_;

	vkAllocateCommandBuffers(*device_, &allocInfo, command_buffer_);
}

void StartCommandBuffer(VkCommandBuffer* command_buffer_,
                        VkCommandBufferUsageFlagBits command_buffer_usage_)
{
	VkCommandBufferBeginInfo beginInfo = {};
	beginInfo.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO;
	beginInfo.flags = command_buffer_usage_;

	vkBeginCommandBuffer(*command_buffer_, &beginInfo);
}

void SubmitCommandBuffer(VkCommandBuffer* command_buffer_, uint8 command_buffer_count_,
                         VkQueue* queue_, VkFence* fence_)
{
	VkSubmitInfo submitInfo = {};
	submitInfo.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO;
	submitInfo.commandBufferCount = command_buffer_count_;
	submitInfo.pCommandBuffers = command_buffer_;

	vkQueueSubmit(*queue_, 1, &submitInfo, *fence_);
}

void CreateBuffer(VkDevice* device_, VkBuffer* buffer_, VkDeviceSize buffer_size_, VkBufferUsageFlagBits buffer_usage_,
                  VkSharingMode buffer_sharing_mode_)
{
	VkBufferCreateInfo bufferInfo = {};
	bufferInfo.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
	bufferInfo.size = buffer_size_;
	bufferInfo.usage = buffer_usage_;
	bufferInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	GS_VK_CHECK(vkCreateBuffer(*device_, &bufferInfo, ALLOCATOR, buffer_), "Failed to create buffer!");
}

static void CreateVkImage(VkDevice* device_, VkImage* image_, Extent2D image_extent_, VkFormat image_format_,
                          VkImageTiling image_tiling_, int image_usage_)
{
	VkImageCreateInfo imageInfo = {};
	imageInfo.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
	imageInfo.imageType = VK_IMAGE_TYPE_2D;
	imageInfo.extent.width = image_extent_.Width;
	imageInfo.extent.height = image_extent_.Height;
	imageInfo.extent.depth = 1;
	imageInfo.mipLevels = 1;
	imageInfo.arrayLayers = 1;
	imageInfo.format = image_format_;
	imageInfo.tiling = image_tiling_;
	imageInfo.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
	imageInfo.usage = image_usage_;
	imageInfo.samples = VK_SAMPLE_COUNT_1_BIT;
	imageInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

	GS_VK_CHECK(vkCreateImage(*device_, &imageInfo, ALLOCATOR, image_), "failed to create image!");
}

void TransitionImageLayout(VkDevice* device_, VkImage* image_, VkFormat image_format_,
                           VkImageLayout from_image_layout_, VkImageLayout to_image_layout_,
                           VkCommandBuffer* command_buffer_)
{
	VkImageMemoryBarrier barrier = {};
	barrier.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
	barrier.oldLayout = from_image_layout_;
	barrier.newLayout = to_image_layout_;
	barrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
	barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
	barrier.image = *image_;
	barrier.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	barrier.subresourceRange.baseMipLevel = 0;
	barrier.subresourceRange.levelCount = 1;
	barrier.subresourceRange.baseArrayLayer = 0;
	barrier.subresourceRange.layerCount = 1;

	VkPipelineStageFlags sourceStage;
	VkPipelineStageFlags destinationStage;

	if (from_image_layout_ == VK_IMAGE_LAYOUT_UNDEFINED && to_image_layout_ == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL)
	{
		barrier.srcAccessMask = 0;
		barrier.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;

		sourceStage = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
		destinationStage = VK_PIPELINE_STAGE_TRANSFER_BIT;
	}
	else if (from_image_layout_ == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL && to_image_layout_ ==
		VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL)
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

	vkCmdPipelineBarrier(*command_buffer_, sourceStage, destinationStage, 0, 0, nullptr, 0, nullptr, 1, &barrier);
}

VkFormat VulkanRenderDevice::findSupportedFormat(const DArray<VkFormat>& formats,
                                                 VkFormatFeatureFlags formatFeatureFlags, VkImageTiling imageTiling)
{
	VkFormatProperties format_properties;

	bool isSupported = false;

	for (auto& e : formats)
	{
		vkGetPhysicalDeviceFormatProperties(PhysicalDevice, e, &format_properties);

		switch (imageTiling)
		{
		case VK_IMAGE_TILING_LINEAR:
			isSupported = format_properties.linearTilingFeatures & formatFeatureFlags;
			break;
		case VK_IMAGE_TILING_OPTIMAL:
			isSupported = format_properties.optimalTilingFeatures & formatFeatureFlags;
			break;
		}

		if (isSupported)
		{
			return e;
		}
	}

	return VK_FORMAT_UNDEFINED;
}

void VulkanRenderDevice::allocateMemory(VkMemoryRequirements* memoryRequirements,
                                        VkMemoryPropertyFlagBits memoryPropertyFlag, VkDeviceMemory* deviceMemory)
{
	VkMemoryAllocateInfo vk_memory_allocate_info = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
	vk_memory_allocate_info.allocationSize = memoryRequirements->size;
	vk_memory_allocate_info.memoryTypeIndex = Device.FindMemoryType(memoryRequirements->memoryTypeBits,
	                                                                memoryPropertyFlag);

	GS_VK_CHECK(vkAllocateMemory(Device.GetVkDevice(), &vk_memory_allocate_info, ALLOCATOR, deviceMemory),
	            "Failed to allocate memory!");
}

VulkanRenderDevice::VulkanRenderDevice(const RenderDeviceCreateInfo& renderDeviceCreateInfo) : Instance("Game Studio"),
                                           PhysicalDevice(Instance),
                                           Device(Instance, PhysicalDevice),
                                           TransientCommandPool(CreateCommandPool())
{
	vkGetPhysicalDeviceProperties(PhysicalDevice, &deviceProperties);
}

VulkanRenderDevice::~VulkanRenderDevice()
{
	//vkDeviceWaitIdle(device);
	//vkDestroyDevice(device, ALLOCATOR);
	//DestroyDebugUtilsMessengerEXT(Instance, debugMessenger, ALLOCATOR);
	//vkDestroyInstance(instance, ALLOCATOR);
}

GPUInfo VulkanRenderDevice::GetGPUInfo()
{
	GPUInfo result;

	result.GPUName = deviceProperties.deviceName;
	result.DriverVersion = deviceProperties.vendorID;
	result.APIVersion = deviceProperties.apiVersion;

	return result;
}

RenderMesh* VulkanRenderDevice::CreateMesh(const MeshCreateInfo& _MCI)
{
	return new VulkanMesh(&Device, TransientCommandPool, _MCI.VertexData,
	                      _MCI.VertexCount * _MCI.VertexLayout->GetSize(), _MCI.IndexData, _MCI.IndexCount);
}

UniformBuffer* VulkanRenderDevice::CreateUniformBuffer(const UniformBufferCreateInfo& _BCI)
{
	return new VulkanUniformBuffer(&Device, _BCI);
}

RenderTarget* VulkanRenderDevice::CreateRenderTarget(const RenderTarget::RenderTargetCreateInfo& _ICI)
{
	return new VulkanRenderTarget(this, _ICI);
}

Texture* VulkanRenderDevice::CreateTexture(const TextureCreateInfo& TCI_)
{
	// CREATE STAGING BUFFER (AND DEVICE MEMORY)
// COPY IMAGE DATA TO STAGING BUFFER
// CREATE IMAGE (AND DEVICE MEMORY)
// TRANSITION LAYOUT FROM UNDEFINED TO TRANSFER_DST
// COPY STAGING BUFFER TO IMAGE
// TRANSITION LAYOUT FROM TRANSFER_DST TO { DESIRED USE }

	auto device = Device.GetVkDevice();

	VkBuffer staging_buffer = VK_NULL_HANDLE;
	VkDeviceMemory staging_buffer_memory = VK_NULL_HANDLE;

	DArray<VkFormat> formats = {FormatToVkFormat(TCI_.ImageFormat), VK_FORMAT_R8G8B8A8_UNORM};

	auto originalFormat = FormatToVkFormat(TCI_.ImageFormat);
	auto supportedFormat = findSupportedFormat(formats, VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT, VK_IMAGE_TILING_OPTIMAL);

	uint64 originalTextureSize = TCI_.ImageDataSize;
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

	CreateBuffer(&device, &staging_buffer, supportedTextureSize, VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
	             VK_SHARING_MODE_EXCLUSIVE);

	{
		VkMemoryRequirements memRequirements;
		vkGetBufferMemoryRequirements(Device, staging_buffer, &memRequirements);


		VkMemoryAllocateInfo allocInfo = {};
		allocInfo.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
		allocInfo.allocationSize = memRequirements.size;
		allocInfo.memoryTypeIndex = Device.FindMemoryType(memRequirements.memoryTypeBits,
		                                                  VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT |
		                                                  VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);


		GS_VK_CHECK(vkAllocateMemory(Device, &allocInfo, nullptr, &staging_buffer_memory),
		            "failed to allocate buffer memory!");

		vkBindBufferMemory(Device, staging_buffer, staging_buffer_memory, 0);
	}

	void* data = nullptr;
	vkMapMemory(Device, staging_buffer_memory, 0, supportedTextureSize, 0, &data);

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
					memcpy(static_cast<char*>(data) + i, static_cast<char*>(TCI_.ImageData) + i_n, 3);
					static_cast<char*>(data)[i + 3] = 0;
				}

				break;
			}
		}
	}
	else
	{
		supportedTextureSize = originalTextureSize;
		memcpy(data, TCI_.ImageData, static_cast<size_t>(supportedTextureSize));
	}


	vkUnmapMemory(Device, staging_buffer_memory);


	VkImage image = VK_NULL_HANDLE;
	VkDeviceMemory image_memory = VK_NULL_HANDLE;

	CreateVkImage(&device, &image, TCI_.Extent, supportedFormat, VK_IMAGE_TILING_OPTIMAL,
	              VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_SAMPLED_BIT);

	{
		VkMemoryRequirements memRequirements;
		vkGetImageMemoryRequirements(Device, image, &memRequirements);

		VkMemoryAllocateInfo allocInfo = {};
		allocInfo.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
		allocInfo.allocationSize = memRequirements.size;
		allocInfo.memoryTypeIndex = Device.FindMemoryType(memRequirements.memoryTypeBits,
		                                                  VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

		GS_VK_CHECK(vkAllocateMemory(Device, &allocInfo, nullptr, &image_memory), "failed to allocate buffer memory!");

		vkBindImageMemory(Device, image, image_memory, 0);
	}

	VkCommandBuffer commandBuffer = VK_NULL_HANDLE;

	ImageTransferCommandPool = TransientCommandPool.GetHandle();

	AllocateCommandBuffer(&device, &ImageTransferCommandPool, &commandBuffer, VK_COMMAND_BUFFER_LEVEL_PRIMARY, 1);

	StartCommandBuffer(&commandBuffer, VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT);

	TransitionImageLayout(&device, &image, supportedFormat, VK_IMAGE_LAYOUT_UNDEFINED,
	                      VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &commandBuffer);

	VkBufferImageCopy region = {};
	region.bufferOffset = 0;
	region.bufferRowLength = 0;
	region.bufferImageHeight = 0;
	region.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	region.imageSubresource.mipLevel = 0;
	region.imageSubresource.baseArrayLayer = 0;
	region.imageSubresource.layerCount = 1;
	region.imageOffset = {0, 0, 0};
	region.imageExtent = {TCI_.Extent.Width, TCI_.Extent.Height, 1};

	vkCmdCopyBufferToImage(commandBuffer, staging_buffer, image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &region);

	TransitionImageLayout(&device, &image, supportedFormat, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
	                      ImageLayoutToVkImageLayout(TCI_.Layout), &commandBuffer);

	vkEndCommandBuffer(commandBuffer);

	auto queue = Device.GetTransferQueue().GetVkQueue();
	VkFence fence = nullptr;

	SubmitCommandBuffer(&commandBuffer, 1, &queue, &fence);

	vkQueueWaitIdle(queue);
	vkFreeCommandBuffers(Device, ImageTransferCommandPool, 1, &commandBuffer);

	vkDestroyBuffer(Device, staging_buffer, ALLOCATOR);
	vkFreeMemory(Device, staging_buffer_memory, ALLOCATOR);


	VkImageViewCreateInfo viewInfo = {};
	viewInfo.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO;
	viewInfo.image = image;
	viewInfo.viewType = VK_IMAGE_VIEW_TYPE_2D;
	viewInfo.format = supportedFormat;
	viewInfo.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	viewInfo.subresourceRange.baseMipLevel = 0;
	viewInfo.subresourceRange.levelCount = 1;
	viewInfo.subresourceRange.baseArrayLayer = 0;
	viewInfo.subresourceRange.layerCount = 1;

	VkImageView imageView;

	GS_VK_CHECK(vkCreateImageView(device, &viewInfo, nullptr, &imageView), "failed to create texture image view!");


	VkSamplerCreateInfo samplerInfo = {};
	samplerInfo.sType = VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO;
	samplerInfo.magFilter = VK_FILTER_LINEAR;
	samplerInfo.minFilter = VK_FILTER_LINEAR;
	samplerInfo.addressModeU = VK_SAMPLER_ADDRESS_MODE_REPEAT;
	samplerInfo.addressModeV = VK_SAMPLER_ADDRESS_MODE_REPEAT;
	samplerInfo.addressModeW = VK_SAMPLER_ADDRESS_MODE_REPEAT;

	samplerInfo.anisotropyEnable = VkBool32(TCI_.Anisotropy);
	samplerInfo.maxAnisotropy = static_cast<float>(TCI_.Anisotropy == 0 ? 1 : TCI_.Anisotropy);

	samplerInfo.borderColor = VK_BORDER_COLOR_INT_OPAQUE_BLACK;
	samplerInfo.unnormalizedCoordinates = VK_FALSE;
	samplerInfo.compareEnable = VK_FALSE;
	samplerInfo.compareOp = VK_COMPARE_OP_ALWAYS;
	samplerInfo.mipmapMode = VK_SAMPLER_MIPMAP_MODE_LINEAR;
	samplerInfo.mipLodBias = 0.0f;
	samplerInfo.minLod = 0.0f;
	samplerInfo.maxLod = 0.0f;

	VkSampler textureSampler;
	GS_VK_CHECK(vkCreateSampler(device, &samplerInfo, nullptr, &textureSampler), "failed to create texture sampler!");

	VulkanTextureCreateInfo vulkan_texture_create_info;
	vulkan_texture_create_info.TextureImage = image;
	vulkan_texture_create_info.TextureImageMemory = image_memory;
	vulkan_texture_create_info.TextureImageView = imageView;
	vulkan_texture_create_info.TextureSampler = textureSampler;

	return new VulkanTexture(TCI_, vulkan_texture_create_info);
}

BindingsPool* VulkanRenderDevice::CreateBindingsPool(const BindingsPoolCreateInfo& bindingsPoolCreateInfo)
{
	return new VulkanBindingsPool(this, bindingsPoolCreateInfo);
}

BindingsSet* VulkanRenderDevice::CreateBindingsSet(const BindingsSetCreateInfo& bindingsSetCreateInfo)
{
	return new VulkanBindingsSet(this, bindingsSetCreateInfo);
}

GraphicsPipeline* VulkanRenderDevice::CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI)
{
	return new VulkanGraphicsPipeline(_GPCI);
}

RAPI::RenderPass* VulkanRenderDevice::CreateRenderPass(const RenderPassCreateInfo& _RPCI)
{
	return new VulkanRenderPass(&Device, _RPCI.Descriptor);
}

ComputePipeline* VulkanRenderDevice::CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI)
{
	return new ComputePipeline();
}

Framebuffer* VulkanRenderDevice::CreateFramebuffer(const FramebufferCreateInfo& _FCI)
{
	return new VulkanFramebuffer(this, _FCI);
}

RenderContext* VulkanRenderDevice::CreateRenderContext(const RenderContextCreateInfo& _RCCI)
{
	return new VulkanRenderContext(this, _RCCI);
}

VulkanRenderDevice::VulkanQueue::VulkanQueue(const QueueCreateInfo& queueCreateInfo, const VulkanQueueCreateInfo& vulkanQueueCreateInfo) : queue(vulkanQueueCreateInfo.Queue), queueIndex(vulkanQueueCreateInfo.QueueIndex), familyIndex(vulkanQueueCreateInfo.FamilyIndex)
{
}
