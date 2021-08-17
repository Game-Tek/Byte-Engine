#pragma once

#include "GAL/RenderDevice.h"

#include "DX12.h"

#include "GTSL/Delegate.hpp"
#include "GTSL/Range.h"
#include <dxgi1_6.h>

namespace GAL
{
	class DX12RenderDevice : public RenderDevice
	{
	public:		
		void Initialize(const CreateInfo& info) {
			IDXGIFactory4* factory4; GTSL::uint32 factoryFlags = 0;

			if constexpr (_DEBUG) {
				if (info.Debug) {
					factoryFlags |= DXGI_CREATE_FACTORY_DEBUG;
				}
			}

			DX_CHECK(CreateDXGIFactory2(factoryFlags, IID_IDXGIFactory4, reinterpret_cast<void**>(&factory4)))

			IDXGIAdapter1* adapter1 = nullptr;
			IDXGIAdapter4* adapter4 = nullptr;

			{
				SIZE_T maxDedicatedVideoMemory = 0;
				for (UINT i = 0; factory4->EnumAdapters1(i, &adapter1) != DXGI_ERROR_NOT_FOUND; ++i) {
					DXGI_ADAPTER_DESC1 dxgiAdapterDesc1;
					adapter1->GetDesc1(&dxgiAdapterDesc1);

					// Check to see if the adapter can create a D3D12 device without actually 
					// creating it. The adapter with the largest dedicated video memory
					// is favored.
					if ((dxgiAdapterDesc1.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) == 0 && SUCCEEDED(D3D12CreateDevice(adapter1, D3D_FEATURE_LEVEL_12_1, IID_ID3D12Device2, nullptr)) && dxgiAdapterDesc1.DedicatedVideoMemory > maxDedicatedVideoMemory)
					{
						maxDedicatedVideoMemory = dxgiAdapterDesc1.DedicatedVideoMemory;
						DX_CHECK(adapter1->QueryInterface(IID_IDXGIAdapter4, reinterpret_cast<void**>(&adapter4))) //BUG: CHECK QUERY INTERFACE
					}
				}

				DX_CHECK(D3D12CreateDevice(adapter4, D3D_FEATURE_LEVEL_12_1, IID_ID3D12Device2, reinterpret_cast<void**>(&device)))
				//setName(device, info);
			}

			if constexpr (_DEBUG) {
				ID3D12InfoQueue* infoQueue;
				DX_CHECK(device->QueryInterface(IID_ID3D12InfoQueue, reinterpret_cast<void**>(&infoQueue)));

				infoQueue->SetBreakOnSeverity(D3D12_MESSAGE_SEVERITY_CORRUPTION, true);
				infoQueue->SetBreakOnSeverity(D3D12_MESSAGE_SEVERITY_ERROR, true);
				infoQueue->SetBreakOnSeverity(D3D12_MESSAGE_SEVERITY_WARNING, true);

				// Suppress whole categories of messages
				//D3D12_MESSAGE_CATEGORY Categories[] = {};

				// Suppress messages based on their severity level
				D3D12_MESSAGE_SEVERITY severities[] = {
					D3D12_MESSAGE_SEVERITY_INFO
				};

				// Suppress individual messages by their ID
				D3D12_MESSAGE_ID denyIds[] = {
					D3D12_MESSAGE_ID_CLEARRENDERTARGETVIEW_MISMATCHINGCLEARVALUE,   // I'm really not sure how to avoid this message.
					D3D12_MESSAGE_ID_MAP_INVALID_NULLRANGE,                         // This warning occurs when using capture frame while graphics debugging.
					D3D12_MESSAGE_ID_UNMAP_INVALID_NULLRANGE,                       // This warning occurs when using capture frame while graphics debugging.
				};

				D3D12_INFO_QUEUE_FILTER infoQueueFilter = {};
				//NewFilter.DenyList.NumCategories = _countof(Categories);
				//NewFilter.DenyList.pCategoryList = Categories;
				infoQueueFilter.DenyList.NumSeverities = _countof(severities);
				infoQueueFilter.DenyList.pSeverityList = severities;
				infoQueueFilter.DenyList.NumIDs = _countof(denyIds);
				infoQueueFilter.DenyList.pIDList = denyIds;

				DX_CHECK(infoQueue->PushStorageFilter(&infoQueueFilter))

				infoQueue->Release();
			}

			for (GTSL::uint32 i = 0; i < info.QueueKeys.ElementCount(); ++i) {
				info.QueueKeys[i].Type = info.Queues[i];
			}
		}

		~DX12RenderDevice() {
			device->Release();
			debug->Release();

			debugClear(device);

			if constexpr (_DEBUG) {
				debugClear(debug);
			}
		}
		
		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(device); }
		[[nodiscard]] ID3D12Device2* GetID3D12Device2() const { return device; }

	private:
		ID3D12Device2* device = nullptr;

#if (_DEBUG)
		ID3D12Debug* debug = nullptr;
#endif
	};
}
