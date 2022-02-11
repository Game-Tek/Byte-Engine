#pragma once

#include "DX12.h"
#include "DX12Buffer.h"
#include "DX12Framebuffer.h"
#include "DX12Pipelines.h"
#include "DX12Texture.h"
#include "DX12RenderDevice.h"
#include "GAL/CommandList.h"

//#include <pix3.h>

#include "GAL/RenderCore.h"

#include <GTSL/RGB.h>
#include <GTSL/Vector.hpp>

#undef MemoryBarrier

namespace GAL {
	class DX12PipelineLayout;
	struct BuildAccelerationStructuresInfo;
	class DX12Pipeline;
	class DX12Queue;

	class DX12CommandList final : public CommandList {
	public:
		DX12CommandList() = default;

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

			commandList->BeginRenderPass(renderPassRenderTargetDescs.GetLength(), renderPassRenderTargetDescs.begin(), &renderPassDepthStencilDesc, D3D12_RENDER_PASS_FLAG_NONE);
		}

		void EndRenderPass(const DX12RenderDevice* renderDevice) {
			commandList->EndRenderPass();
		}

		void ExecuteCommandLists(const DX12RenderDevice* render_device, const GTSL::Range<const DX12CommandList*> command_lists) {
			for(const auto& e : command_lists) {
				commandList->ExecuteBundle(e.GetID3D12CommandList());
			}
		}

		struct MemoryBarrier {
			GTSL::uint32 SourceAccessFlags, DestinationAccessFlags;
		};

		struct BufferBarrier {
			DX12Buffer Buffer;
			AccessType SourceAccessFlags, DestinationAccessFlags;
		};

		struct TextureBarrier {
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
				}
			}

			commandList->ResourceBarrier(resourceBarriers.GetLength(), resourceBarriers.begin());
		}

		void BindPipeline(const DX12RenderDevice* renderDevice, DX12Pipeline pipeline, ShaderStage shaderStage) const {
			commandList->SetPipelineState(pipeline.GetID3D12PipelineState());
		}

		void BindIndexBuffer(const DX12RenderDevice* renderDevice, DX12Buffer buffer, const GTSL::uint32 offset, const GTSL::uint32 indexCount, IndexType indexType) const {
			D3D12_INDEX_BUFFER_VIEW indexBufferView;
			indexBufferView.Format = ToDX12(indexType);
			indexBufferView.BufferLocation = buffer.GetID3D12Resource()->GetGPUVirtualAddress() + offset;
			indexBufferView.SizeInBytes = GAL::IndexSize(indexType);
			commandList->IASetIndexBuffer(&indexBufferView);
		}

		void BindVertexBuffer(const DX12RenderDevice* renderDevice, const DX12Buffer buffer, const GTSL::uint32 size, const GTSL::uint32 offset, const GTSL::uint32 stride) const {
			D3D12_VERTEX_BUFFER_VIEW vertexBufferView;
			vertexBufferView.SizeInBytes = size;
			vertexBufferView.BufferLocation = buffer.GetID3D12Resource()->GetGPUVirtualAddress() + offset;
			vertexBufferView.StrideInBytes = stride;
			commandList->IASetVertexBuffers(0, 1, &vertexBufferView);
		}
		
		void UpdatePushConstant(const DX12RenderDevice* renderDevice, DX12PipelineLayout pipelineLayout, GTSL::uint32 offset, GTSL::Range<const GTSL::byte*> data, ShaderStage shaderStages) {
			if (shaderStages & (ShaderStages::VERTEX | ShaderStages::FRAGMENT | ShaderStages::RAY_GEN)) {
				commandList->SetGraphicsRoot32BitConstants(0, data.Bytes() / 4, data.begin(), offset / 4);
				return;
			}

			if (shaderStages & (ShaderStages::COMPUTE)) {
				commandList->SetComputeRoot32BitConstants(0, data.Bytes() / 4, data.begin(), offset / 4);
				return;
			}
		}
		
		void DrawIndexed(const DX12RenderDevice* renderDevice, uint32_t indexCount, uint32_t instanceCount = 1) const {
			commandList->DrawIndexedInstanced(indexCount, instanceCount, 0, 0, 0);
		}

		void TraceRays(const DX12RenderDevice* renderDevice, GTSL::StaticVector<ShaderTableDescriptor, 4> shaderTableDescriptors, GTSL::Extent3D dispatchSize) {
			D3D12_DISPATCH_RAYS_DESC dispatchRaysDesc;
			dispatchRaysDesc.Width = dispatchSize.Width; dispatchRaysDesc.Height = dispatchSize.Height; dispatchRaysDesc.Depth = dispatchSize.Depth;
			
			dispatchRaysDesc.RayGenerationShaderRecord.StartAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::RAY_GEN_TABLE_INDEX].Address);
			dispatchRaysDesc.RayGenerationShaderRecord.SizeInBytes = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::RAY_GEN_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::RAY_GEN_TABLE_INDEX].EntrySize);
			
			dispatchRaysDesc.HitGroupTable.StartAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::HIT_TABLE_INDEX].Address);
			dispatchRaysDesc.HitGroupTable.SizeInBytes = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::HIT_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::HIT_TABLE_INDEX].EntrySize);
			dispatchRaysDesc.HitGroupTable.StrideInBytes = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::HIT_TABLE_INDEX].EntrySize);
			
			dispatchRaysDesc.MissShaderTable.StartAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::MISS_TABLE_INDEX].Address);
			dispatchRaysDesc.MissShaderTable.SizeInBytes = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::MISS_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::MISS_TABLE_INDEX].EntrySize);
			dispatchRaysDesc.MissShaderTable.StrideInBytes = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::MISS_TABLE_INDEX].EntrySize);
			
			dispatchRaysDesc.CallableShaderTable.StartAddress = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].Address);
			dispatchRaysDesc.CallableShaderTable.SizeInBytes = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].Entries * shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].EntrySize);
			dispatchRaysDesc.CallableShaderTable.StrideInBytes = static_cast<GTSL::uint64>(shaderTableDescriptors[GAL::CALLABLE_TABLE_INDEX].EntrySize);
			
			commandList->DispatchRays(&dispatchRaysDesc);
		}

		void AddLabel(const DX12RenderDevice* renderDevice, GTSL::Range<const char8_t*> name) {}

		void BeginRegion(const DX12RenderDevice* renderDevice) const {}

		void EndRegion(const DX12RenderDevice* renderDevice) const {}
		
		void Dispatch(const DX12RenderDevice* renderDevice, GTSL::Extent3D workGroups) {
			commandList->Dispatch(workGroups.Width, workGroups.Height, workGroups.Depth);
		}

		void BindBindingsSets(const DX12RenderDevice* renderDevice) {
			commandList->SetComputeRootDescriptorTable(0, { 0 });
			commandList->SetComputeRootUnorderedAccessView(0, 0);
			commandList->SetComputeRootConstantBufferView(0, 0);
			commandList->SetComputeRootShaderResourceView(0, 0);
			commandList->SetComputeRootSignature(nullptr);

			commandList->SetGraphicsRootDescriptorTable(0, { 0 });
			commandList->SetGraphicsRootUnorderedAccessView(0, 0);
			commandList->SetGraphicsRootConstantBufferView(0, 0);
			commandList->SetGraphicsRootShaderResourceView(0, 0);
			commandList->SetGraphicsRootSignature(nullptr);
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
			commandList->CopyBufferRegion(destination.GetID3D12Resource(), 0, source.GetID3D12Resource(), 0, size);
		}

		template<class ALLOCATOR>
		void BuildAccelerationStructure(const DX12RenderDevice* renderDevice, const BuildAccelerationStructuresInfo& info) const {
			D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC desc;
			UINT NumPostbuildInfoDescs = 0;
			D3D12_RAYTRACING_ACCELERATION_STRUCTURE_POSTBUILD_INFO_DESC postbuildInfoDescs;

			GTSL::Vector<D3D12_RAYTRACING_GEOMETRY_DESC, ALLOCATOR> geomDesc;

			desc.DestAccelerationStructureData;
			desc.Inputs.DescsLayout;
			desc.Inputs.Flags;
			desc.Inputs.InstanceDescs;
			desc.Inputs.NumDescs = geomDesc.GetLength();
			desc.Inputs.pGeometryDescs = geomDesc.GetData();
			desc.Inputs.Type = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL;
			desc.ScratchAccelerationStructureData;
			desc.SourceAccelerationStructureData;

			postbuildInfoDescs.DestBuffer;
			postbuildInfoDescs.InfoType;

			commandList->BuildRaytracingAccelerationStructure(&desc, NumPostbuildInfoDescs, &postbuildInfoDescs);
		}
		
		~DX12CommandList() = default;

		void Initialize(const DX12RenderDevice* renderDevice, DX12RenderDevice::QueueKey queue, bool isPrimary = true) {
			const D3D12_COMMAND_LIST_TYPE type = isPrimary ? D3D12_COMMAND_LIST_TYPE_DIRECT : D3D12_COMMAND_LIST_TYPE_BUNDLE;

			DX_CHECK(renderDevice->GetID3D12Device2()->CreateCommandAllocator(type, __uuidof(ID3D12CommandAllocator), reinterpret_cast<void**>(commandAllocator)))
			DX_CHECK(renderDevice->GetID3D12Device2()->CreateCommandList(0, type, commandAllocator, nullptr, __uuidof(ID3D12CommandList), reinterpret_cast<void**>(&commandList)))

			//setName(commandAllocator, info);
		}

		[[nodiscard]] ID3D12CommandAllocator* GetID3D12CommandAllocator() const { return commandAllocator; }
		[[nodiscard]] ID3D12GraphicsCommandList5* GetID3D12CommandList() const { return commandList; }

		void Destroy(const DX12RenderDevice* renderDevice) {
			commandAllocator->Release();
			commandList->Release();
			debugClear(commandAllocator);
			debugClear(commandList);
		}
		
	private:
		ID3D12CommandAllocator* commandAllocator = nullptr;
		ID3D12GraphicsCommandList5* commandList = nullptr;
	};
}
