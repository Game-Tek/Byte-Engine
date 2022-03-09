#pragma once

#include "GAL/CommandList.h"

#include "Vulkan.h"
#include "VulkanTexture.h"
#include "VulkanRenderDevice.h"
#include "VulkanAccelerationStructures.h"
#include "VulkanPipelines.h"
#include <GTSL/RGB.hpp>

#include "VulkanBindings.h"
#include "VulkanRenderPass.h"
#include "VulkanSynchronization.h"
#include "GTSL/Vector.hpp"

#undef MemoryBarrier

namespace GAL {
	class VulkanCommandList final : public CommandList {
	public:
		VulkanCommandList() = default;
		
		explicit VulkanCommandList(const VkCommandBuffer commandBuffer) : commandBuffer(commandBuffer) {}

		void Initialize(const VulkanRenderDevice* renderDevice, const GTSL::StringView name, VulkanRenderDevice::QueueKey queueKey, const bool isOptimized = false, const bool isPrimary = true) {
			VkCommandPoolCreateInfo vkCommandPoolCreateInfo{ VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
			vkCommandPoolCreateInfo.queueFamilyIndex = queueKey.Family;
			renderDevice->VkCreateCommandPool(renderDevice->GetVkDevice(), &vkCommandPoolCreateInfo, renderDevice->GetVkAllocationCallbacks(), &commandPool);
			//setName(renderDevice, commandPool, VK_OBJECT_TYPE_COMMAND_POOL, createInfo.Name);

			VkCommandBufferAllocateInfo vkCommandBufferAllocateInfo { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
			vkCommandBufferAllocateInfo.commandPool = commandPool;
			vkCommandBufferAllocateInfo.level = isPrimary ? VK_COMMAND_BUFFER_LEVEL_PRIMARY : VK_COMMAND_BUFFER_LEVEL_SECONDARY;
			vkCommandBufferAllocateInfo.commandBufferCount = 1;

			this->isOptimized = isOptimized;

			renderDevice->VkAllocateCommandBuffers(renderDevice->GetVkDevice(), &vkCommandBufferAllocateInfo, &commandBuffer);
			setName(renderDevice, commandBuffer, VK_OBJECT_TYPE_COMMAND_BUFFER, name);
		}
		
		void BeginRecording(const VulkanRenderDevice* renderDevice) const {
			VkCommandBufferBeginInfo vkCommandBufferBeginInfo{ VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
			vkCommandBufferBeginInfo.pInheritanceInfo = nullptr;
			vkCommandBufferBeginInfo.flags |= isOptimized ? 0 : VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;

			renderDevice->VkResetCommandPool(renderDevice->GetVkDevice(), commandPool, 0);
			renderDevice->VkBeginCommandBuffer(commandBuffer, &vkCommandBufferBeginInfo);
		}

		void EndRecording(const VulkanRenderDevice* renderDevice) const { renderDevice->VkEndCommandBuffer(commandBuffer); }

		void ExecuteCommandLists(const VulkanRenderDevice* render_device, const GTSL::Range<const VulkanCommandList*> command_lists) const {
			GTSL::StaticVector<VkCommandBuffer, 32> vkCommandBuffers;
			for(auto& e : command_lists) { vkCommandBuffers.EmplaceBack(e.GetVkCommandBuffer()); }
			render_device->VkCmdExecuteCommands(commandBuffer, vkCommandBuffers.GetLength(), vkCommandBuffers.GetData());
		}

		//void BeginRenderPass(const VulkanRenderDevice* renderDevice, VulkanRenderPass renderPass, VulkanFramebuffer framebuffer,
		//	GTSL::Extent2D renderArea, GTSL::Range<const RenderPassTargetDescription*> renderPassTargetDescriptions) {
		//	VkRenderPassBeginInfo vkRenderPassBeginInfo{ VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };
		//	vkRenderPassBeginInfo.renderPass = renderPass.GetVkRenderPass();
		//
		//	VkClearValue vkClearValues[32];
		//
		//	for (GTSL::uint8 i = 0; i < static_cast<GTSL::uint8>(renderPassTargetDescriptions.ElementCount()); ++i) {
		//		const auto& color = renderPassTargetDescriptions[i].ClearValue;
		//		vkClearValues[i] = VkClearValue{ color.R(), color.G(), color.B(), color.A() };
		//	}
		//
		//	vkRenderPassBeginInfo.pClearValues = vkClearValues;
		//	vkRenderPassBeginInfo.clearValueCount = static_cast<GTSL::uint32>(renderPassTargetDescriptions.ElementCount());
		//	vkRenderPassBeginInfo.framebuffer = framebuffer.GetVkFramebuffer();
		//	vkRenderPassBeginInfo.renderArea.extent = ToVulkan(renderArea);
		//	vkRenderPassBeginInfo.renderArea.offset = { 0, 0 };
		//
		//	renderDevice->VkCmdBeginRenderPass(commandBuffer, &vkRenderPassBeginInfo, VK_SUBPASS_CONTENTS_INLINE);
		//}
		//
		//void AdvanceSubPass(const VulkanRenderDevice* renderDevice) { renderDevice->VkCmdNextSubpass(commandBuffer, VK_SUBPASS_CONTENTS_INLINE); }
		//
		//void EndRenderPass(const VulkanRenderDevice* renderDevice) { renderDevice->VkCmdEndRenderPass(commandBuffer); }

		void BeginRenderPass(const VulkanRenderDevice* renderDevice, GTSL::Extent2D renderArea, GTSL::Range<const RenderPassTargetDescription*> renderPassTargetDescriptions) {
			GTSL::StaticVector<VkRenderingAttachmentInfoKHR, 16> colorAttachmentInfos; VkRenderingAttachmentInfoKHR depthAttachmentInfo;

			VkRenderingInfoKHR vk_rendering_info_khr{ VK_STRUCTURE_TYPE_RENDERING_INFO_KHR };
			vk_rendering_info_khr.flags = 0;
			vk_rendering_info_khr.layerCount = 1; //TODO: if 0 device lost, validation layers?

			for(auto& e : renderPassTargetDescriptions) {
				VkRenderingAttachmentInfoKHR* attachmentInfo;

				if (e.FormatDescriptor.Type == TextureType::COLOR) {
					attachmentInfo = &colorAttachmentInfos.EmplaceBack();
				} else {
					attachmentInfo = &depthAttachmentInfo;
				}

				attachmentInfo->sType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO_KHR;
				attachmentInfo->pNext = nullptr;
				attachmentInfo->clearValue = { e.ClearValue.R(), e.ClearValue.G(), e.ClearValue.B(), e.ClearValue.A() };
				attachmentInfo->imageLayout = ToVulkan(e.Start, e.FormatDescriptor);
				attachmentInfo->imageView = reinterpret_cast<const VulkanTextureView*>(e.TextureView)->GetVkImageView();
				attachmentInfo->loadOp = ToVkAttachmentLoadOp(e.LoadOperation);
				attachmentInfo->storeOp = ToVkAttachmentStoreOp(e.StoreOperation);
				attachmentInfo->resolveMode = VK_RESOLVE_MODE_NONE;
				attachmentInfo->resolveImageLayout = VK_IMAGE_LAYOUT_UNDEFINED;
				attachmentInfo->resolveImageView = nullptr;
			}

			vk_rendering_info_khr.colorAttachmentCount = colorAttachmentInfos.GetLength();
			vk_rendering_info_khr.pColorAttachments = colorAttachmentInfos.GetData();
			vk_rendering_info_khr.pDepthAttachment = &depthAttachmentInfo;
			vk_rendering_info_khr.pStencilAttachment = nullptr;
			vk_rendering_info_khr.renderArea = { { 0, 0 }, { renderArea.Width, renderArea.Height } };
			vk_rendering_info_khr.viewMask = 0; //multiview
			renderDevice->VkCmdBeginRendering(commandBuffer, &vk_rendering_info_khr);

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

		void EndRenderPass(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkCmdEndRendering(commandBuffer);
		}

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

		void BindIndexBuffer(const VulkanRenderDevice* renderDevice, const VulkanBuffer buffer, GTSL::uint32 offset, [[maybe_unused]] const GTSL::uint32 indexCount, const IndexType indexType) const {
			renderDevice->VkCmdBindIndexBuffer(commandBuffer, buffer.GetVkBuffer(), offset, ToVulkan(indexType));
		}

		void BindVertexBuffers(const VulkanRenderDevice* renderDevice, GTSL::Range<const VulkanBuffer*> buffers, GTSL::Range<const GTSL::uint32*> offsets, [[maybe_unused]] GTSL::uint32 size, [[maybe_unused]] GTSL::uint32 stride) const {
			GTSL::StaticVector<VkBuffer, 16> vkBuffers;
			GTSL::StaticVector<uint64, 16> vkOffsets;

			for(uint32 i = 0; i < buffers.ElementCount(); ++i) {
				vkBuffers.EmplaceBack(buffers[i].GetVkBuffer());
				vkOffsets.EmplaceBack(offsets[i]);
			}

			renderDevice->VkCmdBindVertexBuffers(commandBuffer, 0, vkBuffers.GetLength(), vkBuffers.GetData(), vkOffsets.GetData());
		}

		void UpdatePushConstant(const VulkanRenderDevice* renderDevice, VulkanPipelineLayout pipelineLayout, GTSL::uint32 offset, GTSL::Range<const GTSL::byte*> data, ShaderStage shaderStages) {
			GTSL_ASSERT(data.ElementCount() < 128, "Data size is larger than can be pushed.");
			renderDevice->VkCmdPushConstants(commandBuffer, pipelineLayout.GetVkPipelineLayout(), ToVulkan(shaderStages), offset, static_cast<GTSL::uint32>(data.Bytes()), data.begin());
		}
		
		void Draw(const VulkanRenderDevice* renderDevice, uint32_t vertex_count, uint32_t instance_count = 1) const {
			renderDevice->VkCmdDraw(commandBuffer, vertex_count, instance_count, 0, 0);
		}

		void DrawIndexed(const VulkanRenderDevice* renderDevice, uint32_t indexCount, uint32_t instanceCount, uint32_t first_instance_index, uint32 vertex_offset) const {
			renderDevice->VkCmdDrawIndexed(commandBuffer, indexCount, instanceCount, 0, vertex_offset, first_instance_index);
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
#if BE_DEBUG
			VkDebugUtilsLabelEXT vkLabelInfo{ VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT };
			vkLabelInfo.pLabelName = reinterpret_cast<const char*>(name.GetData());
			renderDevice->vkCmdInsertDebugUtilsLabelEXT(commandBuffer, &vkLabelInfo);
#endif
		}

		void BeginRegion(const VulkanRenderDevice* renderDevice, GTSL::Range<const char8_t*> name) const {
#if BE_DEBUG
			VkDebugUtilsLabelEXT vkLabelInfo{ VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT };
			vkLabelInfo.pLabelName = reinterpret_cast<const char*>(name.GetData());
			renderDevice->vkCmdBeginDebugUtilsLabelEXT(commandBuffer, &vkLabelInfo);
#endif
		}

		void EndRegion(const VulkanRenderDevice* renderDevice) const {
#if BE_DEBUG
			renderDevice->vkCmdEndDebugUtilsLabelEXT(commandBuffer);
#endif
		}
		
		void Dispatch(const VulkanRenderDevice* renderDevice, GTSL::Extent3D workGroups) {
			renderDevice->VkCmdDispatch(commandBuffer, workGroups.Width, workGroups.Height, workGroups.Depth);
		}

		void DispatchIndirect(const VulkanRenderDevice* render_device, const VulkanBuffer buffer, const uint64 offset) {
			render_device->VkCmdDispatchIndirect(commandBuffer, buffer.GetVkBuffer(), offset);
		}

		void BindBindingsSets(const VulkanRenderDevice* renderDevice, ShaderStage shaderStage, GTSL::Range<const VulkanBindingsSet*> bindingsSets, VulkanPipelineLayout pipelineLayout, GTSL::uint32 firstSet) {
			GTSL::StaticVector<VkDescriptorSet, 16> vkDescriptorSets;
			for (auto e : bindingsSets) { vkDescriptorSets.EmplaceBack(e.GetVkDescriptorSet()); }

			const GTSL::uint32 bindingSetCount = static_cast<GTSL::uint32>(bindingsSets.ElementCount());

			if (shaderStage & (ShaderStages::VERTEX | ShaderStages::FRAGMENT | ShaderStages::MESH)) {
				renderDevice->VkCmdBindDescriptorSets(commandBuffer, VK_PIPELINE_BIND_POINT_GRAPHICS, pipelineLayout.GetVkPipelineLayout(), firstSet, bindingSetCount, vkDescriptorSets.begin(), 0, nullptr);
			}
			if (shaderStage & ShaderStages::COMPUTE) {
				renderDevice->VkCmdBindDescriptorSets(commandBuffer, VK_PIPELINE_BIND_POINT_COMPUTE, pipelineLayout.GetVkPipelineLayout(), firstSet, bindingSetCount, vkDescriptorSets.begin(), 0, nullptr);
			}
			if (shaderStage & ShaderStages::RAY_GEN) {
				renderDevice->VkCmdBindDescriptorSets(commandBuffer, VK_PIPELINE_BIND_POINT_RAY_TRACING_KHR, pipelineLayout.GetVkPipelineLayout(), firstSet, bindingSetCount, vkDescriptorSets.begin(), 0, nullptr);
			}
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

		void CopyTextureToTexture(const VulkanRenderDevice* renderDevice, VulkanTexture sourceTexture, VulkanTexture destinationTexture, TextureLayout sourceLayout, TextureLayout destinationLayout, FormatDescriptor sourceFormat, FormatDescriptor destinationFormat, GTSL::Extent3D extent) {
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

			renderDevice->VkCmdCopyImage(commandBuffer, sourceTexture.GetVkImage(), ToVulkan(sourceLayout, sourceFormat), destinationTexture.GetVkImage(), ToVulkan(destinationLayout, destinationFormat), 1, &vkImageCopy);
		}

		void BlitTexture(const VulkanRenderDevice* render_device, const VulkanTexture source_texture, const TextureLayout source_layout, const FormatDescriptor source_format_descriptor, const GTSL::Extent3D source_extent, const VulkanTexture destination_texture, const TextureLayout destination_layout, const FormatDescriptor destination_format_descriptor, const GTSL::Extent3D destination_extent) {
			VkImageBlit2KHR vkImageBlit2Khr{ VK_STRUCTURE_TYPE_IMAGE_BLIT_2_KHR };
			vkImageBlit2Khr.srcOffsets[0] = { 0, 0, 0 };
			vkImageBlit2Khr.srcOffsets[1] = { source_extent.Width, source_extent.Height, source_extent.Depth };
			vkImageBlit2Khr.srcSubresource.mipLevel = 0;
			vkImageBlit2Khr.srcSubresource.baseArrayLayer = 0;
			vkImageBlit2Khr.srcSubresource.layerCount = 1;
			vkImageBlit2Khr.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;

			vkImageBlit2Khr.dstOffsets[0] = { 0, 0, 0 };
			vkImageBlit2Khr.dstOffsets[1] = { destination_extent.Width, destination_extent.Height, destination_extent.Depth };
			vkImageBlit2Khr.dstSubresource.mipLevel = 0;
			vkImageBlit2Khr.dstSubresource.baseArrayLayer = 0;
			vkImageBlit2Khr.dstSubresource.layerCount = 1;
			vkImageBlit2Khr.dstSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;

			VkBlitImageInfo2KHR vkBlitImageInfo2Khr{ VK_STRUCTURE_TYPE_BLIT_IMAGE_INFO_2_KHR };
			vkBlitImageInfo2Khr.srcImage = source_texture.GetVkImage();
			vkBlitImageInfo2Khr.srcImageLayout = ToVulkan(source_layout, source_format_descriptor);
			vkBlitImageInfo2Khr.dstImage = destination_texture.GetVkImage();
			vkBlitImageInfo2Khr.dstImageLayout = ToVulkan(destination_layout, destination_format_descriptor);
			vkBlitImageInfo2Khr.filter = VK_FILTER_LINEAR;
			vkBlitImageInfo2Khr.regionCount = 1;
			vkBlitImageInfo2Khr.pRegions = &vkImageBlit2Khr;
			render_device->VkCmdBlitImage2KHR(commandBuffer, &vkBlitImageInfo2Khr);
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
		void AddPipelineBarrier(const VulkanRenderDevice* renderDevice, GTSL::Range<const BarrierData*> barriers, const ALLOCATOR& allocator) const {
			GTSL::Vector<VkImageMemoryBarrier2KHR, ALLOCATOR> imageMemoryBarriers(4, allocator); GTSL::Vector<VkMemoryBarrier2KHR, ALLOCATOR> memoryBarriers(4, allocator); GTSL::Vector<VkBufferMemoryBarrier2KHR, ALLOCATOR> bufferBarriers(4, allocator);

			for(auto& b : barriers) {
				switch (b.Type) {
				case BarrierType::MEMORY: {
					auto& barrier = b.Memory;
					VkMemoryBarrier2KHR& memoryBarrier = memoryBarriers.EmplaceBack();

					memoryBarrier.sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2_KHR; memoryBarrier.pNext = nullptr;
					memoryBarrier.srcAccessMask = ToVulkan(b.SourceAccess, b.SourceStage);
					memoryBarrier.dstAccessMask = ToVulkan(b.DestinationAccess, b.DestinationStage);
					memoryBarrier.srcStageMask = ToVulkan(b.SourceStage);
					memoryBarrier.dstStageMask = ToVulkan(b.DestinationStage);
						
					break;
				}
				case BarrierType::BUFFER: {
					auto& barrier = b.Buffer;
					VkBufferMemoryBarrier2KHR& bufferBarrier = bufferBarriers.EmplaceBack();

					bufferBarrier.sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER_2_KHR; bufferBarrier.pNext = nullptr;
					bufferBarrier.size = barrier.Size;
					bufferBarrier.buffer = static_cast<const VulkanBuffer*>(barrier.Buffer)->GetVkBuffer();
					bufferBarrier.srcAccessMask = ToVulkan(b.SourceAccess, b.SourceStage);
					bufferBarrier.dstAccessMask = ToVulkan(b.DestinationAccess, b.DestinationStage);
					bufferBarrier.srcQueueFamilyIndex = b.From;
					bufferBarrier.dstQueueFamilyIndex = b.To;
					bufferBarrier.srcStageMask = ToVulkan(b.SourceStage);
					bufferBarrier.dstStageMask = ToVulkan(b.DestinationStage);
					break;
				}
				case BarrierType::TEXTURE: {
					auto& barrier = b.Texture;
					VkImageMemoryBarrier2KHR& textureBarrier = imageMemoryBarriers.EmplaceBack();
						
					textureBarrier.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2_KHR; textureBarrier.pNext = nullptr;
					textureBarrier.oldLayout = ToVulkan(barrier.CurrentLayout, barrier.Format);
					textureBarrier.newLayout = ToVulkan(barrier.TargetLayout, barrier.Format);
					textureBarrier.srcQueueFamilyIndex = b.From;
					textureBarrier.dstQueueFamilyIndex = b.To;
					textureBarrier.image = static_cast<const VulkanTexture*>(barrier.Texture)->GetVkImage();
					textureBarrier.subresourceRange.aspectMask = ToVulkan(barrier.Format.Type);
					textureBarrier.subresourceRange.baseMipLevel = 0;
					textureBarrier.subresourceRange.levelCount = 1;
					textureBarrier.subresourceRange.baseArrayLayer = 0;
					textureBarrier.subresourceRange.layerCount = 1;
					textureBarrier.srcStageMask = ToVulkan(b.SourceStage);
					textureBarrier.dstStageMask = ToVulkan(b.DestinationStage);
					textureBarrier.srcAccessMask = ToVulkan(b.SourceAccess, b.SourceStage, barrier.Format);
					textureBarrier.dstAccessMask = ToVulkan(b.DestinationAccess, b.DestinationStage, barrier.Format);
					break;
				}
				}
			}

			VkDependencyInfoKHR vk_dependency_info_khr{ VK_STRUCTURE_TYPE_DEPENDENCY_INFO_KHR };
			vk_dependency_info_khr.bufferMemoryBarrierCount = bufferBarriers.GetLength();
			vk_dependency_info_khr.pBufferMemoryBarriers = bufferBarriers.GetData();
			vk_dependency_info_khr.imageMemoryBarrierCount = imageMemoryBarriers.GetLength();
			vk_dependency_info_khr.pImageMemoryBarriers = imageMemoryBarriers.GetData();
			vk_dependency_info_khr.memoryBarrierCount = memoryBarriers.GetLength();
			vk_dependency_info_khr.pMemoryBarriers = memoryBarriers.GetData();
			vk_dependency_info_khr.dependencyFlags = 0;

			renderDevice->VkCmdPipelineBarrier2(commandBuffer, &vk_dependency_info_khr);
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
		void BuildAccelerationStructure(const VulkanRenderDevice* renderDevice, GTSL::Range<const AccelerationStructureBuildInfo*> infos, const ALLOCATOR& allocator) const {
			GTSL::Vector<VkAccelerationStructureBuildGeometryInfoKHR, ALLOCATOR> buildGeometryInfos(infos.ElementCount(), allocator);
			GTSL::Vector<GTSL::Vector<VkAccelerationStructureGeometryKHR, ALLOCATOR>, ALLOCATOR> geoPerAccStructure(infos.ElementCount(), allocator);
			GTSL::Vector<GTSL::Vector<VkAccelerationStructureBuildRangeInfoKHR, ALLOCATOR>, ALLOCATOR> buildRangesPerAccelerationStructure(infos.ElementCount(), allocator);
			GTSL::Vector<VkAccelerationStructureBuildRangeInfoKHR*, ALLOCATOR> buildRangesRangePerAccelerationStructure(infos.ElementCount(), allocator);

			for (GTSL::uint32 accStrInfoIndex = 0; accStrInfoIndex < infos.ElementCount(); ++accStrInfoIndex) {
				auto& source = infos[accStrInfoIndex];

				geoPerAccStructure.EmplaceBack(source.Geometries.ElementCount(), allocator);
				buildRangesPerAccelerationStructure.EmplaceBack(source.Geometries.ElementCount(), allocator);
				buildRangesRangePerAccelerationStructure.EmplaceBack(buildRangesPerAccelerationStructure[accStrInfoIndex].begin());

				for (GTSL::uint32 i = 0; i < source.Geometries.ElementCount(); ++i) {
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

		void SetEvent(const VulkanRenderDevice* renderDevice, VulkanSynchronizer event, PipelineStage pipelineStage) {
			renderDevice->VkCmdSetEvent(commandBuffer, event.GetVkEvent(), ToVulkan(pipelineStage));
		}

		void ResetEvent(const VulkanRenderDevice* renderDevice, VulkanSynchronizer event, PipelineStage pipelineStage) {
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
		bool isOptimized = false;
	};
}
