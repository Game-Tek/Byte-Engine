#pragma once

#include "GAL/CommandList.h"

#include "Vulkan.h"
#include "VulkanTexture.h"
#include "VulkanRenderDevice.h"
#include "VulkanAccelerationStructures.h"
#include "VulkanPipelines.h"
#include <GTSL/RGB.h>

#include "VulkanBindings.h"
#include "VulkanFramebuffer.h"
#include "VulkanRenderPass.h"
#include "VulkanSynchronization.h"
#include "GTSL/Vector.hpp"

#undef MemoryBarrier

namespace GAL
{
	class VulkanCommandList final : public CommandList
	{
	public:
		VulkanCommandList() = default;
		
		explicit VulkanCommandList(const VkCommandBuffer commandBuffer) : commandBuffer(commandBuffer) {}

		void Initialize(const VulkanRenderDevice* renderDevice, VulkanRenderDevice::QueueKey queueKey, const bool isPrimary = true) {
			VkCommandPoolCreateInfo vkCommandPoolCreateInfo{ VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
			vkCommandPoolCreateInfo.queueFamilyIndex = queueKey.Family;
			renderDevice->VkCreateCommandPool(renderDevice->GetVkDevice(), &vkCommandPoolCreateInfo, renderDevice->GetVkAllocationCallbacks(), &commandPool);
			//setName(renderDevice, commandPool, VK_OBJECT_TYPE_COMMAND_POOL, createInfo.Name);

			VkCommandBufferAllocateInfo vkCommandBufferAllocateInfo { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
			vkCommandBufferAllocateInfo.commandPool = commandPool;
			vkCommandBufferAllocateInfo.level = isPrimary ? VK_COMMAND_BUFFER_LEVEL_PRIMARY : VK_COMMAND_BUFFER_LEVEL_SECONDARY;
			vkCommandBufferAllocateInfo.commandBufferCount = 1;

			renderDevice->VkAllocateCommandBuffers(renderDevice->GetVkDevice(), &vkCommandBufferAllocateInfo, &commandBuffer);
			//setName(allocateCommandBuffersInfo.RenderDevice, vkCommandBuffer[i], VK_OBJECT_TYPE_COMMAND_BUFFER, allocateCommandBuffersInfo.CommandBufferCreateInfos[i].Name);
		}
		
		void BeginRecording(const VulkanRenderDevice* renderDevice) const {
			VkCommandBufferBeginInfo vkCommandBufferBeginInfo{ VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
			//Hint to primary buffer if this is secondary.
			//vk_command_buffer_begin_info.pInheritanceInfo = static_cast<VulkanCommandBuffer*>(beginRecordingInfo.PrimaryCommandBuffer)->GetVkCommandBuffer();
			vkCommandBufferBeginInfo.pInheritanceInfo = nullptr;
			vkCommandBufferBeginInfo.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;

			renderDevice->VkResetCommandPool(renderDevice->GetVkDevice(), commandPool, 0);
			renderDevice->VkBeginCommandBuffer(commandBuffer, &vkCommandBufferBeginInfo);
		}

		void EndRecording(const VulkanRenderDevice* renderDevice) const { renderDevice->VkEndCommandBuffer(commandBuffer); }

		void BeginRenderPass(const VulkanRenderDevice* renderDevice, VulkanRenderPass renderPass, VulkanFramebuffer framebuffer,
			GTSL::Extent2D renderArea, GTSL::Range<const RenderPassTargetDescription*> renderPassTargetDescriptions) {
			VkRenderPassBeginInfo vkRenderPassBeginInfo{ VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };
			vkRenderPassBeginInfo.renderPass = renderPass.GetVkRenderPass();

			VkClearValue vkClearValues[32];

			for (GTSL::uint8 i = 0; i < static_cast<GTSL::uint8>(renderPassTargetDescriptions.ElementCount()); ++i) {
				const auto& color = renderPassTargetDescriptions[i].ClearValue;
				vkClearValues[i] = VkClearValue{ color.R(), color.G(), color.B(), color.A() };
			}

			vkRenderPassBeginInfo.pClearValues = vkClearValues;
			vkRenderPassBeginInfo.clearValueCount = static_cast<GTSL::uint32>(renderPassTargetDescriptions.ElementCount());
			vkRenderPassBeginInfo.framebuffer = framebuffer.GetVkFramebuffer();
			vkRenderPassBeginInfo.renderArea.extent = ToVulkan(renderArea);
			vkRenderPassBeginInfo.renderArea.offset = { 0, 0 };

			renderDevice->VkCmdBeginRenderPass(commandBuffer, &vkRenderPassBeginInfo, VK_SUBPASS_CONTENTS_INLINE);

			VkViewport viewport;
			viewport.x = 0;
			viewport.y = 0;
			viewport.minDepth = 0;
			viewport.maxDepth = 1.0f;
			viewport.width = renderArea.Width;
			viewport.height = renderArea.Height;
			renderDevice->VkCmdSetViewport(commandBuffer, 0, 1, &viewport);

			VkRect2D scissor;
			scissor.extent.width = renderArea.Width;
			scissor.extent.height = renderArea.Height;
			scissor.offset = { 0, 0 };
			renderDevice->VkCmdSetScissor(commandBuffer, 0, 1, &scissor);
		}

		void AdvanceSubPass(const VulkanRenderDevice* renderDevice) { renderDevice->VkCmdNextSubpass(commandBuffer, VK_SUBPASS_CONTENTS_INLINE); }

		void EndRenderPass(const VulkanRenderDevice* renderDevice) { renderDevice->VkCmdEndRenderPass(commandBuffer); }
		
		void BindPipeline(const VulkanRenderDevice* renderDevice, VulkanPipeline pipeline, ShaderStage shaderStage) const {
			VkPipelineBindPoint pipelineBindPoint;
			
			if (shaderStage & (ShaderStages::VERTEX | ShaderStages::FRAGMENT)) {
				pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
			} else {
				if(shaderStage & ShaderStages::COMPUTE) {
					pipelineBindPoint = VK_PIPELINE_BIND_POINT_COMPUTE;
				} else {
					pipelineBindPoint = VK_PIPELINE_BIND_POINT_RAY_TRACING_KHR;
				}
			}

			renderDevice->VkCmdBindPipeline(commandBuffer, pipelineBindPoint, pipeline.GetVkPipeline());
		}

		void BindIndexBuffer(const VulkanRenderDevice* renderDevice, const VulkanBuffer buffer, [[maybe_unused]] GTSL::uint32 size, const GTSL::uint32 offset, const IndexType indexType) const {
			renderDevice->VkCmdBindIndexBuffer(commandBuffer, buffer.GetVkBuffer(), offset, ToVulkan(indexType));
		}

		void BindVertexBuffer(const VulkanRenderDevice* renderDevice, const VulkanBuffer buffer, [[maybe_unused]] GTSL::uint32 size, const GTSL::uint32 offset, [[maybe_unused]] GTSL::uint32 stride) const {
			auto vkBuffer = buffer.GetVkBuffer();
			GTSL::uint64 bigOffset = offset;
			renderDevice->VkCmdBindVertexBuffers(commandBuffer, 0, 1, &vkBuffer, &bigOffset);
		}

		void UpdatePushConstant(const VulkanRenderDevice* renderDevice, VulkanPipelineLayout pipelineLayout, GTSL::uint32 offset, GTSL::Range<const GTSL::byte*> data, ShaderStage shaderStages) {
			GTSL_ASSERT(data.ElementCount() < 128, "Data size is larger than can be pushed.");
			renderDevice->VkCmdPushConstants(commandBuffer, pipelineLayout.GetVkPipelineLayout(), ToVulkan(shaderStages), offset, static_cast<GTSL::uint32>(data.Bytes()), data.begin());
		}
		
		void DrawIndexed(const VulkanRenderDevice* renderDevice, uint32_t indexCount, uint32_t instanceCount = 0) const {
			renderDevice->VkCmdDrawIndexed(commandBuffer, indexCount, instanceCount, 0, 0, 0);
		}

		void DrawMesh(const VulkanRenderDevice* renderDevice, GTSL::uint32 taskCount) const {
			renderDevice->VkCmdDrawMeshTasks(commandBuffer, taskCount, 0);
		}
		
		void TraceRays(const VulkanRenderDevice* renderDevice, GTSL::StaticVector<ShaderTableDescriptor, 4> shaderTableDescriptors, GTSL::Extent3D dispatchSize) {
			VkStridedDeviceAddressRegionKHR raygenSBT, hitSBT, missSBT, callableSBT;
			raygenSBT.deviceAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[RAY_GEN_TABLE_INDEX].Address);
			raygenSBT.size = shaderTableDescriptors[RAY_GEN_TABLE_INDEX].Entries * shaderTableDescriptors[RAY_GEN_TABLE_INDEX].EntrySize;
			raygenSBT.stride = shaderTableDescriptors[RAY_GEN_TABLE_INDEX].EntrySize;

			hitSBT.deviceAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[HIT_TABLE_INDEX].Address);
			hitSBT.size = shaderTableDescriptors[HIT_TABLE_INDEX].Entries * shaderTableDescriptors[HIT_TABLE_INDEX].EntrySize;
			hitSBT.stride = shaderTableDescriptors[HIT_TABLE_INDEX].EntrySize;

			missSBT.deviceAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[MISS_TABLE_INDEX].Address);
			missSBT.size = shaderTableDescriptors[MISS_TABLE_INDEX].Entries * shaderTableDescriptors[MISS_TABLE_INDEX].EntrySize;
			missSBT.stride = shaderTableDescriptors[MISS_TABLE_INDEX].EntrySize;

			callableSBT.deviceAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[CALLABLE_TABLE_INDEX].Address);
			callableSBT.size = shaderTableDescriptors[CALLABLE_TABLE_INDEX].Entries * shaderTableDescriptors[CALLABLE_TABLE_INDEX].EntrySize;
			callableSBT.stride = shaderTableDescriptors[CALLABLE_TABLE_INDEX].EntrySize;

			renderDevice->vkCmdTraceRaysKHR(commandBuffer, &raygenSBT, &missSBT, &hitSBT, &callableSBT, dispatchSize.Width, dispatchSize.Height, dispatchSize.Depth);
		}		
		
		void AddLabel(const VulkanRenderDevice* renderDevice, GTSL::Range<const char8_t*> name) const {
			VkDebugUtilsLabelEXT vkLabelInfo{ VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT };
			vkLabelInfo.pLabelName = reinterpret_cast<const char*>(name.begin());
			renderDevice->vkCmdInsertDebugUtilsLabelEXT(commandBuffer, &vkLabelInfo);
		}

		void BeginRegion(const VulkanRenderDevice* renderDevice, GTSL::Range<const char8_t*> name) const {
			VkDebugUtilsLabelEXT vkLabelInfo{ VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT };
			vkLabelInfo.pLabelName = reinterpret_cast<const char*>(name.begin());
			renderDevice->vkCmdBeginDebugUtilsLabelEXT(commandBuffer, &vkLabelInfo);
		}

		void EndRegion(const VulkanRenderDevice* renderDevice) const { renderDevice->vkCmdEndDebugUtilsLabelEXT(commandBuffer); }
		
		void Dispatch(const VulkanRenderDevice* renderDevice, GTSL::Extent3D workGroups) {
			renderDevice->VkCmdDispatch(commandBuffer, workGroups.Width, workGroups.Height, workGroups.Depth);
		}

		void BindBindingsSets(const VulkanRenderDevice* renderDevice, ShaderStage shaderStage, GTSL::Range<const VulkanBindingsSet*> bindingsSets, GTSL::Range<const GTSL::uint32*> offsets, VulkanPipelineLayout pipelineLayout, GTSL::uint32 firstSet) {

			GTSL::StaticVector<VkDescriptorSet, 16> vkDescriptorSets;
			for (auto e : bindingsSets) { vkDescriptorSets.EmplaceBack(e.GetVkDescriptorSet()); }

			const GTSL::uint32 bindingSetCount = static_cast<GTSL::uint32>(bindingsSets.ElementCount()), offsetCount = static_cast<GTSL::uint32>(offsets.ElementCount());
			
			if (shaderStage & (ShaderStages::VERTEX | ShaderStages::FRAGMENT | ShaderStages::MESH)) {
				renderDevice->VkCmdBindDescriptorSets(commandBuffer, VK_PIPELINE_BIND_POINT_GRAPHICS, pipelineLayout.GetVkPipelineLayout(), firstSet, bindingSetCount,
					reinterpret_cast<const VkDescriptorSet*>(bindingsSets.begin()), offsetCount, offsets.begin());
			}
			
			if (shaderStage & ShaderStages::COMPUTE) {
				renderDevice->VkCmdBindDescriptorSets(commandBuffer, VK_PIPELINE_BIND_POINT_COMPUTE, pipelineLayout.GetVkPipelineLayout(), firstSet, bindingSetCount,
					reinterpret_cast<const VkDescriptorSet*>(bindingsSets.begin()), offsetCount, offsets.begin());
			}
			
			if(shaderStage & ShaderStages::RAY_GEN) {
				renderDevice->VkCmdBindDescriptorSets(commandBuffer, VK_PIPELINE_BIND_POINT_RAY_TRACING_KHR, pipelineLayout.GetVkPipelineLayout(), firstSet, bindingSetCount,
					reinterpret_cast<const VkDescriptorSet*>(bindingsSets.begin()), offsetCount, offsets.begin());
			}
		}

		void CopyTextureToTexture(const VulkanRenderDevice* renderDevice, VulkanTexture sourceTexture,
		                          VulkanTexture destinationTexture, TextureLayout sourceLayout,
		                          TextureLayout destinationLayout, FormatDescriptor sourceFormat,
		                          FormatDescriptor destinationFormat, GTSL::Extent3D extent) {
			VkImageCopy vkImageCopy;
			vkImageCopy.extent = ToVulkan(extent);
			vkImageCopy.srcOffset = {};
			vkImageCopy.dstOffset = {};
			vkImageCopy.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
			vkImageCopy.srcSubresource.baseArrayLayer = 0;
			vkImageCopy.srcSubresource.layerCount = 1;
			vkImageCopy.srcSubresource.mipLevel = 0;

			vkImageCopy.dstSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
			vkImageCopy.dstSubresource.baseArrayLayer = 0;
			vkImageCopy.dstSubresource.layerCount = 1;
			vkImageCopy.dstSubresource.mipLevel = 0;

			renderDevice->VkCmdCopyImage(commandBuffer, sourceTexture.GetVkImage(), ToVulkan(sourceLayout, sourceFormat),
				destinationTexture.GetVkImage(), ToVulkan(destinationLayout, destinationFormat), 1, &vkImageCopy);
		}

		void CopyBufferToTexture(const VulkanRenderDevice* renderDevice, VulkanBuffer source, VulkanTexture destination, const TextureLayout layout, const FormatDescriptor format, GTSL::Extent3D extent) {
			VkBufferImageCopy region;
			region.bufferOffset = 0;
			region.bufferRowLength = 0;
			region.bufferImageHeight = 0;
			region.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
			region.imageSubresource.mipLevel = 0;
			region.imageSubresource.baseArrayLayer = 0;
			region.imageSubresource.layerCount = 1;
			region.imageOffset = VkOffset3D{ 0, 0, 0 };
			region.imageExtent = ToVulkan(extent);
			renderDevice->VkCmdCopyBufferToImage(commandBuffer, source.GetVkBuffer(), destination.GetVkImage(),ToVulkan(layout, format), 1, &region);
		}

		template<class ALLOCATOR>
		void AddPipelineBarrier(const VulkanRenderDevice* renderDevice, GTSL::Range<const BarrierData*> barriers, PipelineStage initialStage, PipelineStage finalStage, const ALLOCATOR& allocator) const
		{
			GTSL::Vector<VkImageMemoryBarrier, ALLOCATOR> imageMemoryBarriers(4, allocator);
			GTSL::Vector<VkMemoryBarrier, ALLOCATOR> memoryBarriers(4, allocator);
			GTSL::Vector<VkBufferMemoryBarrier, ALLOCATOR> bufferBarriers(4, allocator);

			for(auto& b : barriers)
			{
				switch (b.Type)
				{
				case BarrierType::MEMORY: {
					auto& barrier = b.Memory;
					auto& memoryBarrier = memoryBarriers.EmplaceBack();

					memoryBarrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER; memoryBarrier.pNext = nullptr;
					memoryBarrier.srcAccessMask = ToVulkan(barrier.SourceAccess, initialStage);
					memoryBarrier.dstAccessMask = ToVulkan(barrier.DestinationAccess, finalStage);
						
					break;
				}
				case BarrierType::BUFFER: {
					auto& barrier = b.Buffer;
					auto& bufferBarrier = bufferBarriers.EmplaceBack();

					bufferBarrier.sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER; bufferBarrier.pNext = nullptr;
					bufferBarrier.size = barrier.Size;
					bufferBarrier.buffer = static_cast<const VulkanBuffer*>(barrier.Buffer)->GetVkBuffer();
					bufferBarrier.srcAccessMask = ToVulkan(barrier.SourceAccess, initialStage);
					bufferBarrier.dstAccessMask = ToVulkan(barrier.DestinationAccess, finalStage);
					bufferBarrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
					bufferBarrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
					break;
				}
				case BarrierType::TEXTURE: {
					auto& barrier = b.Texture;
					auto& textureBarrier = imageMemoryBarriers.EmplaceBack();
						
					textureBarrier.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER; textureBarrier.pNext = nullptr;
					textureBarrier.oldLayout = ToVulkan(barrier.CurrentLayout, barrier.Format);
					textureBarrier.newLayout = ToVulkan(barrier.TargetLayout, barrier.Format);
					textureBarrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
					textureBarrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
					textureBarrier.image = static_cast<const VulkanTexture*>(barrier.Texture)->GetVkImage();
					textureBarrier.subresourceRange.aspectMask = ToVulkan(barrier.Format.Type);
					textureBarrier.subresourceRange.baseMipLevel = 0;
					textureBarrier.subresourceRange.levelCount = 1;
					textureBarrier.subresourceRange.baseArrayLayer = 0;
					textureBarrier.subresourceRange.layerCount = 1;
					textureBarrier.srcAccessMask = ToVulkan(barrier.SourceAccess, initialStage, barrier.Format);
					textureBarrier.dstAccessMask = ToVulkan(barrier.DestinationAccess, finalStage, barrier.Format);
					break;
				}
				}
			}

			renderDevice->VkCmdPipelineBarrier(commandBuffer, ToVulkan(initialStage), ToVulkan(finalStage), 0,
				memoryBarriers.GetLength(), memoryBarriers.begin(),
				bufferBarriers.GetLength(), bufferBarriers.begin(),
				imageMemoryBarriers.GetLength(), imageMemoryBarriers.begin());
		}
		
		void CopyBuffer(const VulkanRenderDevice* renderDevice, VulkanBuffer source, VulkanBuffer destination, const GTSL::uint32 size) const {
			VkBufferCopy vkBufferCopy;
			vkBufferCopy.size = size; vkBufferCopy.srcOffset = 0; vkBufferCopy.dstOffset = 0;
			renderDevice->VkCmdCopyBuffer(commandBuffer, source.GetVkBuffer(), destination.GetVkBuffer(), 1, &vkBufferCopy);
		}

		void CopyBuffer(const VulkanRenderDevice* renderDevice, VulkanBuffer source, const GTSL::uint32 sOffset, VulkanBuffer destination, const GTSL::uint32 dOffset, const GTSL::uint32 size) const {
			VkBufferCopy vkBufferCopy;
			vkBufferCopy.size = size; vkBufferCopy.srcOffset = sOffset; vkBufferCopy.dstOffset = dOffset;
			renderDevice->VkCmdCopyBuffer(commandBuffer, source.GetVkBuffer(), destination.GetVkBuffer(), 1, &vkBufferCopy);
		}

		template<typename ALLOCATOR>
		void BuildAccelerationStructure(const VulkanRenderDevice* renderDevice, GTSL::Range<BuildAccelerationStructureInfo*> infos, const ALLOCATOR& allocator) const
		{
			GTSL::Vector<VkAccelerationStructureBuildGeometryInfoKHR, ALLOCATOR> buildGeometryInfos(infos.ElementCount(), allocator);
			GTSL::Vector<GTSL::Vector<VkAccelerationStructureGeometryKHR, ALLOCATOR>, ALLOCATOR> geoPerAccStructure(infos.ElementCount(), allocator);
			GTSL::Vector<GTSL::Vector<VkAccelerationStructureBuildRangeInfoKHR, ALLOCATOR>, ALLOCATOR> buildRangesPerAccelerationStructure(infos.ElementCount(), allocator);
			GTSL::Vector<VkAccelerationStructureBuildRangeInfoKHR*, ALLOCATOR> buildRangesRangePerAccelerationStructure(infos.ElementCount(), allocator);

			for (GTSL::uint32 accStrInfoIndex = 0; accStrInfoIndex < infos.ElementCount(); ++accStrInfoIndex) {
				auto& source = infos[accStrInfoIndex];

				geoPerAccStructure.EmplaceBack(source.Geometries.ElementCount(), allocator);
				buildRangesPerAccelerationStructure.EmplaceBack(source.Geometries.ElementCount(), allocator);
				buildRangesRangePerAccelerationStructure.EmplaceBack(buildRangesPerAccelerationStructure[accStrInfoIndex].begin());

				for (GTSL::uint32 i = 0; i < source.Geometries.ElementCount(); ++i)
				{
					VkAccelerationStructureGeometryKHR accelerationStructureGeometry; VkAccelerationStructureBuildRangeInfoKHR buildRange;
					buildGeometryAndRange(source.Geometries[i], accelerationStructureGeometry, buildRange);
					geoPerAccStructure[accStrInfoIndex].EmplaceBack(accelerationStructureGeometry);
					buildRangesPerAccelerationStructure[accStrInfoIndex].EmplaceBack(buildRange);
				}

				VkAccelerationStructureBuildGeometryInfoKHR buildGeometryInfo{ VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR };
				buildGeometryInfo.flags = source.Flags;
				buildGeometryInfo.srcAccelerationStructure = source.SourceAccelerationStructure.GetVkAccelerationStructure();
				buildGeometryInfo.dstAccelerationStructure = source.DestinationAccelerationStructure.GetVkAccelerationStructure();
				buildGeometryInfo.type = source.Geometries[0].Type == GeometryType::INSTANCES ? VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR : VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;
				buildGeometryInfo.pGeometries = geoPerAccStructure[accStrInfoIndex].begin();
				buildGeometryInfo.ppGeometries = nullptr;
				buildGeometryInfo.geometryCount = geoPerAccStructure[accStrInfoIndex].GetLength();
				buildGeometryInfo.scratchData.deviceAddress = static_cast<GTSL::uint64>(source.ScratchBufferAddress);
				buildGeometryInfo.mode = source.SourceAccelerationStructure.GetVkAccelerationStructure() ? VK_BUILD_ACCELERATION_STRUCTURE_MODE_UPDATE_KHR : VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
				buildGeometryInfos.EmplaceBack(buildGeometryInfo);
			}

			renderDevice->vkCmdBuildAccelerationStructuresKHR(commandBuffer, buildGeometryInfos.GetLength(),
				buildGeometryInfos.begin(), buildRangesRangePerAccelerationStructure.begin());
		}

		void SetEvent(const VulkanRenderDevice* renderDevice, VulkanEvent event, PipelineStage pipelineStage) {
			renderDevice->VkCmdSetEvent(commandBuffer, event.GetVkEvent(), ToVulkan(pipelineStage));
		}
		void ResetEvent(const VulkanRenderDevice* renderDevice, VulkanEvent event, PipelineStage pipelineStage) {
			renderDevice->VkCmdResetEvent(commandBuffer, event.GetVkEvent(), ToVulkan(pipelineStage));
		}
		
		[[nodiscard]] VkCommandBuffer GetVkCommandBuffer() const { return commandBuffer; }
		
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyCommandPool(renderDevice->GetVkDevice(), commandPool, renderDevice->GetVkAllocationCallbacks());
			debugClear(commandPool);
		}

		[[nodiscard]] VkCommandPool GetVkCommandPool() const { return commandPool; }
		
	private:
		VkCommandPool commandPool = nullptr;
		VkCommandBuffer commandBuffer = nullptr;
	};
}
