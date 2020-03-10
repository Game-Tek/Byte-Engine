#pragma once

#include "Core.h"

#include "RAPI/RenderDevice.h"

#include "VulkanFramebuffer.h"
#include "VulkanRenderContext.h"

#include "Vulkan.h"

class VulkanRenderDevice final : public RenderDevice
{
public:
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
		~VulkanQueue() = default;

		VkQueue GetVkQueue() const { return queue; }
		uint32 GetQueueIndex() const { return queueIndex; }
	};

private:
#ifdef GS_DEBUG
	PFN_vkCreateDebugUtilsMessengerEXT createDebugUtilsFunction = nullptr;
	VkDebugUtilsMessengerEXT debugMessenger = nullptr;
	PFN_vkDestroyDebugUtilsMessengerEXT destroyDebugUtilsFunction = nullptr;
#endif

	VkInstance instance = nullptr;
	VkPhysicalDevice physicalDevice = nullptr;
	VkDevice device = nullptr;

	FVector<VulkanQueue> vulkanQueues;

	VkAllocationCallbacks allocationCallbacks;

	VkPhysicalDeviceProperties deviceProperties;
	VkPhysicalDeviceMemoryProperties memoryProperties;

public:
	VulkanRenderDevice(const RenderDeviceCreateInfo& renderDeviceCreateInfo);
	~VulkanRenderDevice();

	static bool IsVulkanSupported();

	GPUInfo GetGPUInfo() override;

	RenderMesh* CreateRenderMesh(const RenderMesh::RenderMeshCreateInfo& _MCI) override;
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

	uint32 FindMemoryType(uint32 memoryType, uint32 memoryFlags) const;
	VkFormat FindSupportedFormat(const DArray<VkFormat>& formats, VkFormatFeatureFlags formatFeatureFlags, VkImageTiling imageTiling);

	[[nodiscard]] const VkPhysicalDeviceProperties& GetPhysicalDeviceProperties() const { return deviceProperties; }
	void AllocateMemory(VkMemoryRequirements* memoryRequirements, VkMemoryPropertyFlagBits memoryPropertyFlag, VkDeviceMemory* deviceMemory);

	void AllocateAndBindBuffer();
	void AllocateAndBindImage();

	VkAllocationCallbacks* GetVkAllocationCallbacks() const { return nullptr; }
};
