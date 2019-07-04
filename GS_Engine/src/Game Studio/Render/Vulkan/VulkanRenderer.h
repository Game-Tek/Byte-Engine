#pragma once

#include "Core.h"

#include "..\Renderer.h"

#include "VulkanBase.h"

class VulkanRenderer final : public Renderer
{
	Vulkan_Instance Instance;
	Vulkan_Device Device;
public:
	VulkanRenderer();
	~VulkanRenderer();

	RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCI) final override;
	Shader* CreateShader(const ShaderCreateInfo& _SI) final override;
	Buffer* CreateBuffer(const BufferCreateInfo& _BCI) final override;
	GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) final override;
	RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) final override;

	INLINE const Vulkan_Device& GetVulkanDevice() const { return Device; }
};

MAKE_VK_HANDLE(VkInstance)

GS_CLASS Vulkan_Instance
{
	VkInstance Instance = nullptr;
public:
	Vulkan_Instance(const char* _AppName);
	~Vulkan_Instance();

	INLINE VkInstance GetVkInstance() const { return Instance; }
};

MAKE_VK_HANDLE(VkQueue)
struct VkDeviceQueueCreateInfo;
struct QueueInfo;

GS_CLASS Vulkan_Device
{
	VkDevice Device = nullptr;
	VkQueue GraphicsQueue;
	VkQueue ComputeQueue;
	VkQueue TransferQueue;
	VkPhysicalDevice PhysicalDevice = nullptr;

	static void CreateQueueInfo(QueueInfo& _DQCI, VkPhysicalDevice _PD);
	static void CreatePhysicalDevice(VkPhysicalDevice& _PD, VkInstance _Instance);
	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _Type);
public:
	Vulkan_Device(VkInstance _Instance);
	~Vulkan_Device();

	uint32 FindMemoryType(uint32 _TypeFilter, VkMemoryPropertyFlags _Properties) const;
	INLINE VkDevice GetVkDevice() const { return Device; }
	INLINE VkPhysicalDevice GetVkPhysicalDevice() const { return PhysicalDevice; }
};