#pragma once

#include "GAL/RenderCore.h"

#include "DX12.h"
#include "DX12Memory.h"
#include "DX12RenderDevice.h"

namespace GAL
{
	struct MemoryRequirements;

	class DX12Buffer final
	{
	public:
		DX12Buffer() = default;
		
		void GetMemoryRequirements(const DX12RenderDevice* renderDevice, GTSL::uint32 size, BufferUse bufferType, MemoryRequirements* memoryRequirements) {
			D3D12_RESOURCE_DESC resourceDesc;
			resourceDesc.Dimension = D3D12_RESOURCE_DIMENSION_BUFFER;
			resourceDesc.Width = size;
			resourceDesc.Height = 1;
			resourceDesc.DepthOrArraySize = 1;

			D3D12_RESOURCE_ALLOCATION_INFO allocInfo = renderDevice->GetID3D12Device2()->GetResourceAllocationInfo(0, 1, &resourceDesc);

			memoryRequirements->Alignment = static_cast<GTSL::uint32>(allocInfo.Alignment);
			memoryRequirements->Size = static_cast<GTSL::uint32>(allocInfo.SizeInBytes);
			memoryRequirements->MemoryTypes = 0;
		}

		void Initialize(const DX12RenderDevice* renderDevice, const MemoryRequirements& memoryRequirements, DX12Memory memory, BufferUse bufferType, GTSL::uint32 offset) {
			D3D12_RESOURCE_DESC resourceDesc;
			resourceDesc.Dimension = D3D12_RESOURCE_DIMENSION_BUFFER;
			resourceDesc.Width = memoryRequirements.Size;
			resourceDesc.Height = 1;
			resourceDesc.DepthOrArraySize = 1;
			renderDevice->GetID3D12Device2()->CreatePlacedResource(memory.GetID3D12Heap(), offset, &resourceDesc, ToDX12(bufferType), nullptr, __uuidof(ID3D12Resource), reinterpret_cast<void**>(&resource));
		}

		[[nodiscard]] ID3D12Resource* GetID3D12Resource() const { return resource; }
		GTSL::uint64 GetAddress() const { return static_cast<GTSL::uint64>(resource->GetGPUVirtualAddress()); }
		GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(resource); }
		
		void Destroy(const DX12RenderDevice* renderDevice) {
			resource->Release();
			debugClear(resource);
		}
		
		~DX12Buffer() = default;
		
	private:
		ID3D12Resource* resource = nullptr;
	};
}
