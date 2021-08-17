#pragma once

#include "DX12.h"
#include "DX12RenderDevice.h"

namespace GAL
{
	class DX12Memory final
	{
	public:
		DX12Memory() = default;

		void Initialize(const DX12RenderDevice* renderDevice, const GTSL::Range<const char8_t*> name, AllocationFlag flags, GTSL::uint32 size, MemoryType memoryType) {
			D3D12_HEAP_DESC heapDesc;
			heapDesc.Flags = D3D12_HEAP_FLAGS(11); //D3D12_HEAP_FLAG_CREATE_NOT_ZEROED
			heapDesc.Alignment = 1024;
			heapDesc.SizeInBytes = size;
			heapDesc.Properties.CreationNodeMask = 0;
			heapDesc.Properties.VisibleNodeMask = 0;
			heapDesc.Properties.Type;
			heapDesc.Properties.CPUPageProperty;
			heapDesc.Properties.MemoryPoolPreference;

			DX_CHECK(renderDevice->GetID3D12Device2()->CreateHeap(&heapDesc, __uuidof(ID3D12Heap), reinterpret_cast<void**>(&heap)));
			setName(heap, name);
		}
		
		void Destroy(const DX12RenderDevice* renderDevice) {
			heap->Release();
			debugClear(heap);
		}
		
		[[nodiscard]] ID3D12Heap* GetID3D12Heap() const { return heap; }
		
		~DX12Memory() = default;
		
	private:
		ID3D12Heap* heap = nullptr;
	};
}
