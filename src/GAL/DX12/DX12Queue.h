#pragma once

#include "GAL/Queue.h"
#include "DX12CommandBuffer.h"
#include "DX12Synchronization.h"

namespace GAL {
	class DX12Queue final : public Queue
	{
	public:
		DX12Queue() = default;

		void Initialize(const DX12RenderDevice* renderDevice, DX12RenderDevice::QueueKey queueKey) {
			D3D12_COMMAND_QUEUE_DESC desc;
			desc.Type = ToDX12(queueKey.QueueType);
			desc.Priority = D3D12_COMMAND_QUEUE_PRIORITY_HIGH;
			desc.Flags = D3D12_COMMAND_QUEUE_FLAG_NONE;
			desc.NodeMask = 0;

			DX_CHECK(renderDevice->GetID3D12Device2()->CreateCommandQueue(&desc, __uuidof(ID3D12CommandQueue), reinterpret_cast<void**>(&commandQueue)));
		}
		
		void Submit(const GTSL::Range<const GTSL::Range<const WorkUnit*>*> submitInfos, const DX12Fence fence) const {
			for(auto& s : submitInfos) {
				GTSL::StaticVector<ID3D12CommandList*, 16> commandLists;

				for(auto& e : s) {
					commandLists.EmplaceBack(static_cast<const DX12CommandBuffer*>(e.CommandBuffer)->GetID3D12CommandList());
				}
				
				commandQueue->ExecuteCommandLists(commandLists.GetLength(), commandLists.begin());
			}
		}

		void Wait(const DX12RenderDevice* renderDevice) const {
			ID3D12Fence* fence;

			renderDevice->GetID3D12Device2()->CreateFence(0, D3D12_FENCE_FLAG_NONE, __uuidof(ID3D12Fence),
				reinterpret_cast<void**>(&fence));

			commandQueue->Wait(fence, 1);

			fence->Release();
		}
		
		~DX12Queue() {
			commandQueue->Release();
			debugClear(commandQueue);
		}

		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(commandQueue); }
		[[nodiscard]] ID3D12CommandQueue* GetID3D12CommandQueue() const { return commandQueue; }

	private:
		ID3D12CommandQueue* commandQueue = nullptr;

		friend class DX12RenderDevice;
	};
}
