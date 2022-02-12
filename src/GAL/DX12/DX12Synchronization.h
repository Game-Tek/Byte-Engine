#pragma once

#include "DX12.h"
#include "DX12RenderDevice.h"

#include "GAL/Synchronization.h"

namespace GAL {
	class DX12Synchronizer : public Synchronizer {
	public:
		DX12Synchronizer() = default;
		
		void Initialize(const DX12RenderDevice* renderDevice, Type syncType, bool isSignaled = false, uint64_t initialValue = ~0ULL) {
			SyncType = syncType;

			renderDevice->GetID3D12Device2()->CreateFence(initialValue, D3D12_FENCE_FLAG_NONE, __uuidof(ID3D12Fence), reinterpret_cast<void**>(&fence));
		}

		void Wait(const DX12RenderDevice* renderDevice) const {
			const auto event = CreateEventA(nullptr, false, false, nullptr);
			fence->SetEventOnCompletion(1, event);

			WaitForSingleObject(event, 0xFFFFFFFF);
		}
		
		void Destroy(const DX12RenderDevice* renderDevice) {
			fence->Release();
			debugClear(fence);
		}
		
	private:
		Type SyncType;
		ID3D12Fence* fence = nullptr;

	};
}
