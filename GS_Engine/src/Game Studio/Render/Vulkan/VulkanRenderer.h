#pragma once

#include "Core.h"

#include "Render/Renderer.h"

#include "VulkanBase.h"

#include "VulkanFramebuffer.h"
#include "Vk_Queue.h"
#include "VulkanRenderContext.h"

MAKE_VK_HANDLE(VkInstance)

GS_CLASS Vulkan_Instance
{
	VkInstance Instance = nullptr;
public:
	Vulkan_Instance(const char* _AppName);
	~Vulkan_Instance();

	INLINE VkInstance GetVkInstance() const { return Instance; }

	INLINE operator VkInstance() const
	{
		return Instance;
	}
};

MAKE_VK_HANDLE(VkQueue)
MAKE_VK_HANDLE(VkPhysicalDevice)
struct VkDeviceQueueCreateInfo;
struct QueueInfo;
enum VkPhysicalDeviceType;

GS_CLASS Vulkan_Device
{
	VkDevice Device = nullptr;
	VkPhysicalDevice PhysicalDevice = nullptr;

	Vk_Queue GraphicsQueue;
	Vk_Queue ComputeQueue;
	Vk_Queue TransferQueue;

	void SetVk_Queue(Vk_Queue& _Queue, const uint32 _QueueFamilyIndex);

	static void CreateQueueInfo(QueueInfo& _DQCI, VkPhysicalDevice _PD);
	static void CreatePhysicalDevice(VkPhysicalDevice _PD, VkInstance _Instance);
	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _Type);
public:
	Vulkan_Device(VkInstance _Instance);
	~Vulkan_Device();

	uint32 FindMemoryType(uint32 _TypeFilter, uint32 _Properties) const;
	INLINE VkDevice GetVkDevice() const { return Device; }
	INLINE VkPhysicalDevice GetVkPhysicalDevice() const { return PhysicalDevice; }

	INLINE const Vk_Queue& GetGraphicsQueue() const { return GraphicsQueue; }
	INLINE const Vk_Queue& GetComputeQueue() const { return ComputeQueue; }
	INLINE const Vk_Queue& GetTransferQueue() const { return TransferQueue; }

	INLINE operator VkDevice() const
	{
		return Device;
	}
};

GS_CLASS VulkanRenderer final : public Renderer
{
	Vulkan_Instance Instance;
	Vulkan_Device Device;

	Vk_CommandPool TransientCommandPool;
public:
	VulkanRenderer();
	~VulkanRenderer();

	Shader* CreateShader(const ShaderCreateInfo& _SI) final override;
	Buffer* CreateBuffer(const BufferCreateInfo& _BCI) final override;
	GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) final override;
	RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) final override;
	ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) final override;
	Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) final override;
	RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) final override;

	INLINE const Vulkan_Device& GetVulkanDevice() const { return Device; }
	INLINE VkDevice GetVkDevice() const { return Device.GetVkDevice(); }
};

#define VKRAPI SCAST(VulkanRenderer*, Renderer::GetRenderer())