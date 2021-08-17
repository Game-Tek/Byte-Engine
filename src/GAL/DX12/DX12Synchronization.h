#pragma once

#include "DX12.h"
#include "DX12RenderDevice.h"

namespace GAL
{
	class DX12Fence
	{
	public:
		DX12Fence() = default;
		
		void Initialize(const DX12RenderDevice* renderDevice, const GTSL::uint32 initialValue) {
			renderDevice->GetID3D12Device2()->CreateFence(initialValue, D3D12_FENCE_FLAG_NONE, __uuidof(ID3D12Fence), reinterpret_cast<void**>(&fence));
			//setName(fence, info);
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
		ID3D12Fence* fence = nullptr;
	};

	//class DX12Semaphore
	//{
	//public:
	//
	//private:
	//	ID3D12Semaphore* semaphore = nullptr;
	//};
}
