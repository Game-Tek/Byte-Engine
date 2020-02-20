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

class VulkanRenderDevice final : public RenderDevice
{
	VKInstance Instance;
	vkPhysicalDevice PhysicalDevice;
	VKDevice Device;

	VkCommandPool ImageTransferCommandPool = nullptr;

	VKCommandPool TransientCommandPool;

	VKCommandPoolCreator CreateCommandPool();

	VkPhysicalDeviceProperties deviceProperties;
	VkFormat findSupportedFormat(const DArray<VkFormat>& formats, VkFormatFeatureFlags formatFeatureFlags,
	                             VkImageTiling imageTiling);

protected:
	friend class VulkanTexture;
	friend class VulkanRenderTarget;

	[[nodiscard]] const VkPhysicalDeviceProperties& getPhysicalDeviceProperties() const { return deviceProperties; }
	void allocateMemory(VkMemoryRequirements* memoryRequirements, VkMemoryPropertyFlagBits memoryPropertyFlag,
	                    VkDeviceMemory* deviceMemory);

public:
	VulkanRenderDevice();
	~VulkanRenderDevice();

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

	INLINE VKDevice& GetVKDevice() { return Device; }
	const vkPhysicalDevice& GetPhysicalDevice() const { return PhysicalDevice; }
	[[nodiscard]] VkInstance GetVkInstance() const { return Instance.GetVkInstance(); }
};
