#pragma once

#include "Vulkan.h"
#include "VulkanMemory.h"
#include "VulkanRenderDevice.h"
#include "GAL/Texture.h"

namespace GAL
{	
	class VulkanTexture final : public Texture
	{
	public:
		VulkanTexture() = default;
		VulkanTexture(VkImage i) : image(i) {}

		void GetMemoryRequirements(const VulkanRenderDevice* renderDevice, MemoryRequirements* memoryRequirements, TextureUse uses,
			FormatDescriptor format, GTSL::Extent3D extent, Tiling tiling, GTSL::uint8 mipLevels) {

			VkImageCreateInfo vkImageCreateInfo{ VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO };
			vkImageCreateInfo.imageType = ToVulkanType(extent);
			vkImageCreateInfo.extent = ToVulkan(extent);
			vkImageCreateInfo.mipLevels = mipLevels;
			vkImageCreateInfo.arrayLayers = 1;
			vkImageCreateInfo.format = ToVulkan(MakeFormatFromFormatDescriptor(format));
			vkImageCreateInfo.tiling = ToVulkan(tiling);
			vkImageCreateInfo.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
			vkImageCreateInfo.usage = ToVulkan(uses, format);
			vkImageCreateInfo.samples = VK_SAMPLE_COUNT_1_BIT;
			vkImageCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

			renderDevice->VkCreateImage(renderDevice->GetVkDevice(), &vkImageCreateInfo, renderDevice->GetVkAllocationCallbacks(), &image);

			VkMemoryRequirements vkMemoryRequirements;
			renderDevice->VkGetImageMemoryRequirements(renderDevice->GetVkDevice(), image, &vkMemoryRequirements);
			memoryRequirements->Size = static_cast<GTSL::uint32>(vkMemoryRequirements.size);
			memoryRequirements->Alignment = static_cast<GTSL::uint32>(vkMemoryRequirements.alignment);
			memoryRequirements->MemoryTypes = vkMemoryRequirements.memoryTypeBits;
		}
		
		void Initialize(const VulkanRenderDevice* renderDevice, const VulkanDeviceMemory deviceMemory, const GTSL::uint32 offset) {
			//SET_NAME(image, VK_OBJECT_TYPE_IMAGE, createInfo);
			renderDevice->VkBindImageMemory(renderDevice->GetVkDevice(), image, deviceMemory.GetVkDeviceMemory(), offset);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyImage(renderDevice->GetVkDevice(), image, renderDevice->GetVkAllocationCallbacks());
			debugClear(image);
		}
		
		[[nodiscard]] VkImage GetVkImage() const { return image; }
		
	private:
		VkImage image = nullptr;		
	};

	class VulkanTextureView final
	{
	public:
		VulkanTextureView() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, const GTSL::Range<const char8_t*> name, const VulkanTexture texture, const FormatDescriptor formatDescriptor, const GTSL::Extent3D extent, const GTSL::uint8 mipLevels) {
			VkImageViewCreateInfo vkImageViewCreateInfo{ VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
			vkImageViewCreateInfo.image = texture.GetVkImage();
			vkImageViewCreateInfo.viewType = ToVkImageViewType(extent);
			vkImageViewCreateInfo.components.r = VK_COMPONENT_SWIZZLE_IDENTITY;
			vkImageViewCreateInfo.components.g = VK_COMPONENT_SWIZZLE_IDENTITY;
			vkImageViewCreateInfo.components.b = VK_COMPONENT_SWIZZLE_IDENTITY;
			vkImageViewCreateInfo.components.a = VK_COMPONENT_SWIZZLE_IDENTITY;
			vkImageViewCreateInfo.format = ToVulkan(MakeFormatFromFormatDescriptor(formatDescriptor));
			vkImageViewCreateInfo.subresourceRange.aspectMask = ToVulkan(formatDescriptor.Type);
			vkImageViewCreateInfo.subresourceRange.baseMipLevel = 0;
			vkImageViewCreateInfo.subresourceRange.levelCount = mipLevels;
			vkImageViewCreateInfo.subresourceRange.baseArrayLayer = 0;
			vkImageViewCreateInfo.subresourceRange.layerCount = 1;

			renderDevice->VkCreateImageView(renderDevice->GetVkDevice(), &vkImageViewCreateInfo, renderDevice->GetVkAllocationCallbacks(), &imageView);
			setName(renderDevice, imageView, VK_OBJECT_TYPE_IMAGE_VIEW, name);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyImageView(renderDevice->GetVkDevice(), imageView, renderDevice->GetVkAllocationCallbacks());
			debugClear(imageView);
		}
		
		[[nodiscard]] VkImageView GetVkImageView() const { return imageView; }
		
	private:
		VkImageView imageView = nullptr;

		friend class VulkanRenderContext;
	};

	class VulkanSampler final
	{
	public:
		VulkanSampler() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, const GTSL::uint8 anisotropy) {
			VkSamplerCreateInfo vkSamplerCreateInfo{ VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO };
			vkSamplerCreateInfo.addressModeU = VK_SAMPLER_ADDRESS_MODE_REPEAT;
			vkSamplerCreateInfo.addressModeV = VK_SAMPLER_ADDRESS_MODE_REPEAT;
			vkSamplerCreateInfo.addressModeW = VK_SAMPLER_ADDRESS_MODE_REPEAT;
			vkSamplerCreateInfo.minFilter = VK_FILTER_LINEAR; vkSamplerCreateInfo.magFilter = VK_FILTER_LINEAR;
			vkSamplerCreateInfo.maxAnisotropy = static_cast<GTSL::float32>(anisotropy == 0 ? 1 : anisotropy);
			vkSamplerCreateInfo.anisotropyEnable = static_cast<VkBool32>(anisotropy);
			vkSamplerCreateInfo.borderColor = VK_BORDER_COLOR_INT_OPAQUE_BLACK;
			vkSamplerCreateInfo.mipmapMode = VK_SAMPLER_MIPMAP_MODE_LINEAR;
			vkSamplerCreateInfo.unnormalizedCoordinates = VK_FALSE;
			vkSamplerCreateInfo.compareOp = VK_COMPARE_OP_ALWAYS;
			vkSamplerCreateInfo.compareEnable = VK_FALSE;
			vkSamplerCreateInfo.mipLodBias = 0.0f;
			vkSamplerCreateInfo.minLod = 0.0f;
			vkSamplerCreateInfo.maxLod = 0.0f;

			renderDevice->VkCreateSampler(renderDevice->GetVkDevice(), &vkSamplerCreateInfo, renderDevice->GetVkAllocationCallbacks(), &sampler);
			//setName(renderDevice, sampler, VK_OBJECT_TYPE_SAMPLER, createInfo.Name);
		}
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroySampler(renderDevice->GetVkDevice(), sampler, renderDevice->GetVkAllocationCallbacks());
			debugClear(sampler);
		}

		[[nodiscard]] VkSampler GetVkSampler() const { return sampler; }
	private:
		VkSampler sampler{ nullptr };
	};
}
