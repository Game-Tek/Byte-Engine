#pragma once

#include "DX12.h"
#include "DX12Buffer.h"
#include "DX12Framebuffer.h"
#include "DX12Pipeline.h"
#include "DX12Texture.h"
#include "DX12RenderDevice.h"
#include "GAL/CommandList.h"

#include <GTSL/RGB.h>
#include <GTSL/Vector.hpp>

#undef MemoryBarrier

namespace GAL
{
	class DX12PipelineLayout;
	struct BuildAccelerationStructuresInfo;
	class DX12Pipeline;
	class DX12Queue;

	class DX12CommandBuffer final : public CommandList
	{
	public:
		DX12CommandBuffer() = default;

		void BeginRecording(const DX12RenderDevice* renderDevice) { DX_CHECK(commandAllocator->Reset()) }

		void EndRecording(const DX12RenderDevice* renderDevice) { commandList->Close(); }
		
		void BeginRenderPass(const DX12RenderDevice* renderDevice, DX12RenderPass renderPass, DX12Framebuffer framebuffer,
			GTSL::Extent2D renderArea, GTSL::Range<const RenderPassTargetDescription*> renderPassTargetDescriptions) {
			GTSL::StaticVector<D3D12_RENDER_PASS_RENDER_TARGET_DESC, 16> renderPassRenderTargetDescs;
			D3D12_RENDER_PASS_DEPTH_STENCIL_DESC renderPassDepthStencilDesc;

			for(GTSL::uint8 i = 0; i < renderPassTargetDescriptions.ElementCount(); ++i) {
				if (renderPassTargetDescriptions[i].FormatDescriptor.Type == TextureType::COLOR) {
					auto& e = renderPassRenderTargetDescs.EmplaceBack();
					e.BeginningAccess.Type = ToD3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE(renderPassTargetDescriptions[i].LoadOperation);
					e.BeginningAccess.Clear.ClearValue.Format = ToDX12(MakeFormatFromFormatDescriptor(renderPassTargetDescriptions[i].FormatDescriptor));
					e.BeginningAccess.Clear.ClearValue.Color[0] = renderPassTargetDescriptions[i].ClearValue.R();
					e.BeginningAccess.Clear.ClearValue.Color[1] = renderPassTargetDescriptions[i].ClearValue.G();
					e.BeginningAccess.Clear.ClearValue.Color[2] = renderPassTargetDescriptions[i].ClearValue.B();
					e.BeginningAccess.Clear.ClearValue.Color[3] = renderPassTargetDescriptions[i].ClearValue.A();

					e.EndingAccess.Type = ToD3D12_RENDER_PASS_ENDING_ACCESS_TYPE(renderPassTargetDescriptions[i].StoreOperation);

					e.cpuDescriptor;
				} else {
					renderPassDepthStencilDesc.DepthBeginningAccess.Type = ToD3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE(renderPassTargetDescriptions[i].LoadOperation);
					
					renderPassDepthStencilDesc.DepthBeginningAccess.Clear.ClearValue.Format = ToDX12(MakeFormatFromFormatDescriptor(renderPassTargetDescriptions[i].FormatDescriptor));
					renderPassDepthStencilDesc.DepthBeginningAccess.Clear.ClearValue.DepthStencil.Depth = renderPassTargetDescriptions[i].ClearValue.R();
					
					renderPassDepthStencilDesc.DepthEndingAccess.Type = ToD3D12_RENDER_PASS_ENDING_ACCESS_TYPE(renderPassTargetDescriptions[i].StoreOperation);

					renderPassDepthStencilDesc.cpuDescriptor;
				}
			}
			
			ID3D12GraphicsCommandList4* renderPassCapableCommandList = nullptr;
			commandList->QueryInterface(__uuidof(ID3D12GraphicsCommandList4), reinterpret_cast<void**>(&renderPassCapableCommandList));
			renderPassCapableCommandList->BeginRenderPass(renderPassRenderTargetDescs.GetLength(), renderPassRenderTargetDescs.begin(),
				&renderPassDepthStencilDesc, D3D12_RENDER_PASS_FLAG_NONE);
			renderPassCapableCommandList->Release();
		}

		void EndRenderPass(const DX12RenderDevice* renderDevice) {
			ID3D12GraphicsCommandList4* renderPassCapableCommandList = nullptr;
			commandList->QueryInterface(__uuidof(ID3D12GraphicsCommandList4), reinterpret_cast<void**>(&renderPassCapableCommandList));
			renderPassCapableCommandList->EndRenderPass();
			renderPassCapableCommandList->Release();
		}
		
		struct MemoryBarrier
		{
			GTSL::uint32 SourceAccessFlags, DestinationAccessFlags;
		};

		struct BufferBarrier
		{
			DX12Buffer Buffer;
			AccessType SourceAccessFlags, DestinationAccessFlags;
		};

		struct TextureBarrier
		{
			DX12Texture Texture;

			TextureLayout CurrentLayout, TargetLayout;
			AccessType SourceAccessFlags, DestinationAccessFlags;
		};
		
		template<class ALLOCATOR>
		void AddPipelineBarrier(const DX12RenderDevice* renderDevice, GTSL::Range<const BarrierData*> barriers, ShaderStage initialStage, ShaderStage finalStage, const ALLOCATOR& allocator) {
			GTSL::StaticVector<D3D12_RESOURCE_BARRIER, 64> resourceBarriers;

			for(auto& e : barriers) {
				switch (e.Type) {

				case BarrierType::MEMORY: {
					break;
				}
				case BarrierType::BUFFER: {
					resourceBarriers.EmplaceBack();
					auto& resourceBarrier = resourceBarriers.back();

					resourceBarrier.Flags = D3D12_RESOURCE_BARRIER_FLAG_NONE;
					resourceBarrier.Type = D3D12_RESOURCE_BARRIER_TYPE_TRANSITION;

					resourceBarrier.Transition.StateBefore = ToDX12(e.Barrier.BufferBarrier.SourceAccessFlags);
					resourceBarrier.Transition.StateAfter = ToDX12(e.Barrier.BufferBarrier.DestinationAccessFlags);
					resourceBarrier.Transition.Subresource = 0;
					resourceBarrier.Transition.pResource = static_cast<DX12Buffer*>(e.Barrier.BufferBarrier.Buffer)->GetID3D12Resource();
					break;
				}
				case BarrierType::TEXTURE: {
					resourceBarriers.EmplaceBack();
					auto& resourceBarrier = resourceBarriers.back();

					resourceBarrier.Flags = D3D12_RESOURCE_BARRIER_FLAG_NONE;
					resourceBarrier.Type = D3D12_RESOURCE_BARRIER_TYPE_TRANSITION;

					resourceBarrier.Transition.StateBefore = ToDX12(e.Barrier.TextureBarrier.CurrentLayout);
					resourceBarrier.Transition.StateAfter = ToDX12(e.Barrier.TextureBarrier.TargetLayout);
					resourceBarrier.Transition.Subresource = 0;
					resourceBarrier.Transition.pResource = static_cast<DX12Texture*>(e.Barrier.TextureBarrier.Texture)->GetID3D12Resource();
					break;
				}
				default: ;
				}
			}

			commandList->ResourceBarrier(resourceBarriers.GetLength(), resourceBarriers.begin());
		}

		void BindPipeline(const DX12RenderDevice* renderDevice, DX12Pipeline pipeline, ShaderStage shaderStage) const {
			commandList->SetPipelineState(pipeline.GetID3D12PipelineState());
		}

		void BindIndexBuffer(const DX12RenderDevice* renderDevice, DX12Buffer buffer, const GTSL::uint32 size, const GTSL::uint32 offset, IndexType indexType) const {
			D3D12_INDEX_BUFFER_VIEW indexBufferView;
			indexBufferView.Format = ToDX12(indexType);
			indexBufferView.BufferLocation = buffer.GetID3D12Resource()->GetGPUVirtualAddress() + offset;
			indexBufferView.SizeInBytes = size;
			commandList->IASetIndexBuffer(&indexBufferView);
		}

		void BindVertexBuffer(const DX12RenderDevice* renderDevice, const DX12Buffer buffer, const GTSL::uint32 size, const GTSL::uint32 offset, const GTSL::uint32 stride) const {
			D3D12_VERTEX_BUFFER_VIEW vertexBufferView;
			vertexBufferView.SizeInBytes =size;
			vertexBufferView.BufferLocation = buffer.GetID3D12Resource()->GetGPUVirtualAddress() + offset;
			vertexBufferView.StrideInBytes = stride;
			commandList->IASetVertexBuffers(0, 1, &vertexBufferView);
		}
		
		void UpdatePushConstant(const DX12RenderDevice* renderDevice, DX12PipelineLayout pipelineLayout, GTSL::uint32 offset, GTSL::Range<const GTSL::byte*> data, ShaderStage shaderStages) {
			if (shaderStages & (ShaderStages::VERTEX | ShaderStages::FRAGMENT)) {
				commandList->SetComputeRoot32BitConstants(0, data.Bytes() / 4, data.begin(), offset / 4);
				return;
			}

			if (shaderStages & (ShaderStages::COMPUTE)) {
				commandList->SetGraphicsRoot32BitConstants(0, data.Bytes() / 4, data.begin(), offset / 4);
				return;
			}
		}
		
		void DrawIndexed(const DX12RenderDevice* renderDevice, uint32_t indexCount, uint32_t instanceCount = 0) const {
			commandList->DrawIndexedInstanced(indexCount, instanceCount, 0, 0, 0);
		}

		void TraceRays(const DX12RenderDevice* renderDevice, GTSL::StaticVector<ShaderTableDescriptor, 4> shaderTableDescriptors, GTSL::Extent3D dispatchSize) {
			ID3D12GraphicsCommandList4* t = nullptr;
			commandList->QueryInterface(__uuidof(ID3D12GraphicsCommandList4), reinterpret_cast<void**>(&t));
			D3D12_DISPATCH_RAYS_DESC dispatchRaysDesc;
			dispatchRaysDesc.Width = dispatchSize.Width; dispatchRaysDesc.Height = dispatchSize.Height; dispatchRaysDesc.Depth = dispatchSize.Depth;
			
			dispatchRaysDesc.RayGenerationShaderRecord.StartAddress = shaderTableDescriptors[GAL::RAY_GEN_TABLE_INDEX].Address;
			dispatchRaysDesc.RayGenerationShaderRecord.SizeInBytes = shaderTableDescriptors[GAL::RAY_GEN_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::RAY_GEN_TABLE_INDEX].EntrySize;
			
			dispatchRaysDesc.HitGroupTable.StartAddress = shaderTableDescriptors[GAL::HIT_TABLE_INDEX].Address;
			dispatchRaysDesc.HitGroupTable.SizeInBytes = shaderTableDescriptors[GAL::HIT_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::HIT_TABLE_INDEX].EntrySize;
			dispatchRaysDesc.HitGroupTable.StrideInBytes = shaderTableDescriptors[GAL::HIT_TABLE_INDEX].EntrySize;
			
			dispatchRaysDesc.MissShaderTable.StartAddress = shaderTableDescriptors[GAL::MISS_TABLE_INDEX].Address;
			dispatchRaysDesc.MissShaderTable.SizeInBytes = shaderTableDescriptors[GAL::MISS_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::MISS_TABLE_INDEX].EntrySize;
			dispatchRaysDesc.MissShaderTable.StrideInBytes = shaderTableDescriptors[GAL::MISS_TABLE_INDEX].EntrySize;
			
			dispatchRaysDesc.CallableShaderTable.StartAddress = shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].Address;
			dispatchRaysDesc.CallableShaderTable.SizeInBytes = shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].EntrySize;
			dispatchRaysDesc.CallableShaderTable.StrideInBytes = shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].EntrySize;
			
			t->DispatchRays(&dispatchRaysDesc);

			t->Release();
		}

		void AddLabel(const DX12RenderDevice* renderDevice, GTSL::Range<const char8_t*> name) {
			//commandList->SetMarker(METADA)
		}

		void BeginRegion(const DX12RenderDevice* renderDevice) const;

		void EndRegion(const DX12RenderDevice* renderDevice) const;
		
		void Dispatch(const DX12RenderDevice* renderDevice, GTSL::Extent3D workGroups) {
			commandList->Dispatch(workGroups.Width, workGroups.Height, workGroups.Depth);
		}

		void BindBindingsSets(const DX12RenderDevice* renderDevice) {
		}

		void CopyTextureToTexture(const DX12RenderDevice* renderDevice, const DX12Texture source, const DX12Texture destination, const GTSL::Extent3D extent, const FormatDescriptor format) {
			D3D12_TEXTURE_COPY_LOCATION sourceTextureCopyLocation, destinationTextureCopyLocation;
			sourceTextureCopyLocation.Type = D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
			sourceTextureCopyLocation.pResource = source.GetID3D12Resource();
			sourceTextureCopyLocation.PlacedFootprint.Footprint.Width = extent.Width;
			sourceTextureCopyLocation.PlacedFootprint.Footprint.Height = extent.Height;
			sourceTextureCopyLocation.PlacedFootprint.Footprint.Depth = extent.Depth;
			sourceTextureCopyLocation.PlacedFootprint.Footprint.Format = ToDX12(MakeFormatFromFormatDescriptor(format));
			sourceTextureCopyLocation.PlacedFootprint.Footprint.RowPitch = 0;
			sourceTextureCopyLocation.PlacedFootprint.Offset = 0;

			destinationTextureCopyLocation.Type = D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
			destinationTextureCopyLocation.pResource = destination.GetID3D12Resource();
			destinationTextureCopyLocation.PlacedFootprint.Footprint.Width = extent.Width;
			destinationTextureCopyLocation.PlacedFootprint.Footprint.Height = extent.Height;
			destinationTextureCopyLocation.PlacedFootprint.Footprint.Depth = extent.Depth;
			destinationTextureCopyLocation.PlacedFootprint.Footprint.Format = ToDX12(MakeFormatFromFormatDescriptor(format));
			destinationTextureCopyLocation.PlacedFootprint.Footprint.RowPitch = 0;
			destinationTextureCopyLocation.PlacedFootprint.Offset = 0;

			D3D12_BOX box;
			box.back;
			box.bottom;
			box.front;
			box.left;
			box.right;
			box.top;

			commandList->CopyTextureRegion(&destinationTextureCopyLocation, 0, 0, 0, &sourceTextureCopyLocation, &box);
		}

		void CopyBufferToTexture(const DX12RenderDevice* renderDevice, const DX12Buffer source, const DX12Texture destination, const GTSL::uint32 size) {
			commandList->CopyResource(destination.GetID3D12Resource(), source.GetID3D12Resource());
		}

		void CopyBuffers(const DX12RenderDevice* renderDevice, const DX12Buffer source, const DX12Buffer destination, const GTSL::uint32 size) {
			commandList->CopyBufferRegion(destination.GetID3D12Resource(), 0, source.GetID3D12Resource(),
				0, size);
		}

		void BuildAccelerationStructure(const DX12RenderDevice* renderDevice, const BuildAccelerationStructuresInfo& info) const;
		
		~DX12CommandBuffer() = default;

		void Initialize(const DX12RenderDevice* renderDevice, DX12Queue queue, bool isPrimary = true) {
			const D3D12_COMMAND_LIST_TYPE type = isPrimary ? D3D12_COMMAND_LIST_TYPE_DIRECT : D3D12_COMMAND_LIST_TYPE_BUNDLE;

			DX_CHECK(renderDevice->GetID3D12Device2()->CreateCommandAllocator(type, __uuidof(ID3D12CommandAllocator), reinterpret_cast<void**>(commandAllocator)))
			DX_CHECK(renderDevice->GetID3D12Device2()->CreateCommandList(0, type, commandAllocator, nullptr, __uuidof(ID3D12CommandList), reinterpret_cast<void**>(&commandList)))

			//setName(commandAllocator, info);
		}

		[[nodiscard]] ID3D12CommandAllocator* GetID3D12CommandAllocator() const { return commandAllocator; }
		[[nodiscard]] ID3D12CommandList* GetID3D12CommandList() const { return commandList; }

		void Destroy(const DX12RenderDevice* renderDevice) {
			commandAllocator->Release();
			commandList->Release();
			debugClear(commandAllocator);
			debugClear(commandList);
		}
		
	private:
		ID3D12CommandAllocator* commandAllocator = nullptr;
		ID3D12GraphicsCommandList* commandList = nullptr;
	};
}
