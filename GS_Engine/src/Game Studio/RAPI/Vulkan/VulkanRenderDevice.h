#pragma once

#include "Core.h"

#include "RAPI/RenderDevice.h"

#include "VulkanFramebuffer.h"
#include "VulkanRenderContext.h"

#include "Vulkan.h"

class VulkanRenderDevice final : public RenderDevice
{
#ifdef GS_DEBUG
	PFN_vkCreateDebugUtilsMessengerEXT createDebugUtilsFunction = nullptr;
	VkDebugUtilsMessengerEXT debugMessenger = nullptr;
	PFN_vkDestroyDebugUtilsMessengerEXT destroyDebugUtilsFunction = nullptr;
#endif

	VkInstance instance = nullptr;
	VkPhysicalDevice physicalDevice = nullptr;
	VkDevice device = nullptr;

	VkPhysicalDeviceProperties deviceProperties;
	VkPhysicalDeviceMemoryProperties memoryProperties;
	VkFormat findSupportedFormat(const DArray<VkFormat>& formats, VkFormatFeatureFlags formatFeatureFlags,
	                             VkImageTiling imageTiling);


protected:
	friend class VulkanTexture;
	friend class VulkanRenderTarget;

	[[nodiscard]] const VkPhysicalDeviceProperties& getPhysicalDeviceProperties() const { return deviceProperties; }
	void allocateMemory(VkMemoryRequirements* memoryRequirements, VkMemoryPropertyFlagBits memoryPropertyFlag,
	                    VkDeviceMemory* deviceMemory);

public:
	VulkanRenderDevice(const RenderDeviceCreateInfo& renderDeviceCreateInfo);
	~VulkanRenderDevice();

	class VulkanQueue : public Queue
	{
		VkQueue queue = nullptr;
		uint32 queueIndex = 0;
		uint32 familyIndex = 0;

	public:
		struct VulkanQueueCreateInfo
		{
			VkQueue Queue = nullptr;
			uint32 QueueIndex = 0;
			uint32 FamilyIndex = 0;
		};
		VulkanQueue(const QueueCreateInfo& queueCreateInfo, const VulkanQueueCreateInfo& vulkanQueueCreateInfo);
		~VulkanQueue() = delete;

		VkQueue GetVkQueue() const { return queue; }
		uint32 GetQueueIndex() const { return queueIndex; }
	};

	GPUInfo GetGPUInfo() override;

	RenderMesh* CreateMesh(const MeshCreateInfo& _MCI) override;
	UniformBuffer* CreateUniformBuffer(const UniformBufferCreateInfo& _BCI) override;
	RenderTarget* CreateRenderTarget(const RenderTarget::RenderTargetCreateInfo& _ICI) override;
	Texture* CreateTexture(const TextureCreateInfo& TCI_) override;
	BindingsPool* CreateBindingsPool(const BindingsPoolCreateInfo& bindingsPoolCreateInfo) override;
	BindingsSet* CreateBindingsSet(const BindingsSetCreateInfo& bindingsSetCreateInfo) override;
	GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) override;
	RAPI::RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) override;
	ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) override;
	Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) override;
	RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) override;

	VkInstance GetVkInstance() const { return instance; }
	VkPhysicalDevice GetVkPhysicalDevice() const { return physicalDevice; }
	VkDevice GetVkDevice() const { return device; }

	uint32 findMemorytype(uint32 memoryType, uint32 memoryFlags) const;
};
