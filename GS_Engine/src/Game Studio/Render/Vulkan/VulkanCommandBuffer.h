#pragma once

#include "..\CommandBuffer.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkCommandBuffer)
MAKE_VK_HANDLE(VkCommandPool)

GS_CLASS VulkanCommandBuffer final : public CommandBuffer
{
	Vk_CommandPool CommandPool;
	Vk_CommandBuffer CommandBuffer;
public:
	VulkanCommandBuffer(VkDevice _Device, uint32 _QueueIndex);
	~VulkanCommandBuffer() = default;

	void BeginRecording() final override;
	void EndRecording() final override;
	void BeginRenderPass(const RenderPassBeginInfo& _RPBI) final override;
	void EndRenderPass(RenderPass* _RP) final override;
	void BindGraphicsPipeline(GraphicsPipeline* _GP) final override;
	void BindComputePipeline(ComputePipeline* _CP) final override;
	void DrawIndexed(const DrawInfo& _DI) final override;
};

GS_CLASS Vk_CommandBuffer final : public VulkanObject
{
	VkCommandBuffer CommandBuffer = nullptr;
public:
	Vk_CommandBuffer(VkDevice _Device, VkCommandPool _CP);
	~Vk_CommandBuffer();

	INLINE VkCommandBuffer GetVkCommandBuffer() const { return CommandBuffer; }
};

GS_CLASS Vk_CommandPool final : public VulkanObject
{
	VkCommandPool CommandPool = nullptr;
public:
	Vk_CommandPool(VkDevice _Device, uint32 _QueueIndex);
	~Vk_CommandPool();

	INLINE VkCommandPool GetVkCommandPool() const { return CommandPool; }
};