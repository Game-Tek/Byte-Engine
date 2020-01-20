#pragma once

#include "Core.h"

#include "RAPI/RenderDevice.h"

#include "VulkanFramebuffer.h"
#include "VulkanRenderContext.h"
#include "Native/VKInstance.h"
#include "Native/VKDevice.h"
#include "Native/VKCommandPool.h"
#include "Native/vkPhysicalDevice.h"

#include "Vulkan.h"

struct VkDeviceQueueCreateInfo;
struct QueueInfo;
enum VkPhysicalDeviceType;

class GS_API VulkanRenderDevice final : public RenderDevice
{
	VKInstance Instance;
	vkPhysicalDevice PhysicalDevice;
	VKDevice Device;

	VkCommandPool ImageTransferCommandPool = VK_NULL_HANDLE;
	
	VKCommandPool TransientCommandPool;
	
	VKCommandPoolCreator CreateCommandPool();

	VkPhysicalDeviceProperties deviceProperties;
	bool isImageFormatSupported(VkFormat format, VkFormatFeatureFlags formatFeatureFlags, VkImageTiling imageTiling);

protected:
	friend class VulkanTexture;
	
	[[nodiscard]] const VkPhysicalDeviceProperties& getPhysicalDeviceProperties() const { return deviceProperties; }
	//static void AllocateCommandBuffer(VkDevice* device_, VkCommandPool* command_pool_, VkCommandBuffer* command_buffer_, VkCommandBufferLevel command_buffer_level_, uint8 command_buffer_count_);
	//static void StartCommandBuffer(VkCommandBuffer* command_buffer_, VkCommandBufferUsageFlagBits command_buffer_usage_);
	//static void SubmitCommandBuffer(VkCommandBuffer* command_buffer_, uint8 command_buffer_count_, VkQueue* queue_, VkFence* fence_);
	
	//static void CreateBufferAndMemory(VkDevice* device_, VkBuffer* buffer_, VkDeviceSize buffer_size_, VkBufferUsageFlagBits buffer_usage_, VkSharingMode buffer_sharing_mode_, VkDeviceMemory* device_memory_, VkMemoryPropertyFlags properties_);
	//static void CreateImageAndMemory(VkDevice* device_, VkImage* image_, Extent2D image_extent_, VkFormat image_format_, VkImageTiling image_tiling_, int image_usage_, VkDeviceMemory* device_memory_, VkMemoryPropertyFlagBits properties_);
	//static void TransitionImageLayout(VkDevice* device_, VkImage* image_, VkFormat image_format_, VkImageLayout from_image_layout_, VkImageLayout to_image_layout_, VkCommandBuffer* command_buffer_);
public:
	VulkanRenderDevice();
	~VulkanRenderDevice();

	GPUInfo GetGPUInfo() override;
	
	RenderMesh* CreateMesh(const MeshCreateInfo& _MCI) final override;
	UniformBuffer* CreateUniformBuffer(const UniformBufferCreateInfo& _BCI) final override;
	UniformLayout* CreateUniformLayout(const UniformLayoutCreateInfo& _ULCI) final override;
	Image* CreateImage(const ImageCreateInfo& _ICI) final override;
	Texture* CreateTexture(const TextureCreateInfo& TCI_) override;
	GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) final override;
	RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) final override;
	ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) final override;
	Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) final override;
	RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) final override;

	INLINE const VKDevice& GetVKDevice() const { return Device; }
};