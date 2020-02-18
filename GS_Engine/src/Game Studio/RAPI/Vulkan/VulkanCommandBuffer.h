#pragma once

#include "RAPI/CommandBuffer.h"

#include "Vulkan.h"

class VulkanCommandBuffer : public CommandBuffer
{
	VkCommandBuffer commandBuffer = nullptr;
	
public:
	void BeginRecording(const BeginRecordingInfo& beginRecordingInfo) override;
	void EndRecording(const EndRecordingInfo& endRecordingInfo) override;

	void BeginRenderPass(const BeginRenderPassInfo& beginRenderPassInfo) override;
	void AdvanceSubPass(const AdvanceSubpassInfo& advanceSubpassInfo) override;
	void EndRenderPass(const EndRenderPassInfo& endRenderPassInfo) override;

	void BindGraphicsPipeline(const BindGraphicsPipelineInfo& bindGraphicsPipelineInfo) override;
	void BindComputePipeline(const BindComputePipelineInfo& bindComputePipelineInfo) override;

	void BindMesh(const BindMeshInfo& bindMeshInfo) override;

	void UpdatePushConstant(const UpdatePushConstantsInfo& updatePushConstantsInfo) override;

	void DrawIndexed(const DrawIndexedInfo& drawIndexedInfo) override;
	void Dispatch(const DispatchInfo& dispatchInfo) override;

	void BindBindingsSet(const BindBindingsSetInfo& bindBindingsSetInfo) override;
};
