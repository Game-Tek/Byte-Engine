#pragma once

#include "DX12.h"
#include "DX12AccelerationStructure.hpp"
#include "DX12Buffer.h"
#include "DX12Texture.h"
#include "GAL/Bindings.h"
#include "GAL/DX12/DX12RenderDevice.h"

namespace GAL
{
	class DX12BindingsSet final
	{
	public:
		void Initialize(const DX12RenderDevice* renderDevice)
		{
		}
	
	private:
	};
	
	class DX12BindingsLayout final
	{};

	class DX12BindingsPool final : BindingsPool
	{
	public:
		void Initialize(const DX12RenderDevice* renderDevice, GTSL::Range<const BindingsPoolSize*> bindingsPoolSizes, GTSL::uint32 maxSets) {
			D3D12_DESCRIPTOR_HEAP_DESC descHeapCbvSrv = {};
			descHeapCbvSrv.NumDescriptors = 0;
			
			for (GTSL::uint32 i = 0; i < static_cast<GTSL::uint32>(bindingsPoolSizes.ElementCount()); ++i) {
				//descriptorPoolSize.type = ToVulkan(bindingsPoolSizes[i].BindingType);
				descHeapCbvSrv.NumDescriptors += bindingsPoolSizes[i].Count;
			}

			descHeapCbvSrv.Type = D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV;
			descHeapCbvSrv.Flags = D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE;
			renderDevice->GetID3D12Device2()->CreateDescriptorHeap(&descHeapCbvSrv, __uuidof(ID3D12DescriptorHeap), reinterpret_cast<void**>(&descriptorHeap));
		}

		ID3D12DescriptorHeap* GetID3D12DescriptorHeap() const { return descriptorHeap; }

		struct TextureBindingUpdateInfo {
			DX12Sampler Sampler;
			DX12TextureView TextureView;
			TextureLayout TextureLayout;
			FormatDescriptor FormatDescriptor;
		};

		struct BufferBindingUpdateInfo {
			DX12Buffer Buffer;
			GTSL::uint64 Offset, Range;
		};

		struct AccelerationStructureBindingUpdateInfo {
			DX12AccelerationStructure AccelerationStructure;
		};

		union BindingUpdateInfo
		{
			BindingUpdateInfo(TextureBindingUpdateInfo info) : TextureBindingUpdateInfo(info) {}
			BindingUpdateInfo(BufferBindingUpdateInfo info) : BufferBindingUpdateInfo(info) {}
			BindingUpdateInfo(AccelerationStructureBindingUpdateInfo info) : AccelerationStructureBindingUpdateInfo(info) {}

			TextureBindingUpdateInfo TextureBindingUpdateInfo;
			BufferBindingUpdateInfo BufferBindingUpdateInfo;
			AccelerationStructureBindingUpdateInfo AccelerationStructureBindingUpdateInfo;
		};

		struct BindingsUpdateInfo
		{
			BindingType Type;
			GTSL::uint32 SubsetIndex = 0, BindingIndex = 0;
			GTSL::Range<const BindingUpdateInfo*> BindingUpdateInfos;
		};

		template<class ALLOCATOR>
		void Update(const DX12RenderDevice* renderDevice, const DX12BindingsSet* bindingsSet, GTSL::Range<const BindingsUpdateInfo*> bindingsUpdateInfos, const ALLOCATOR& allocator)
		{
			auto sbv_srv_uav_HeapSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
			auto rtvHeapSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);
			auto dsvHeapSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_DSV);

			auto sbv_srv_uav_handle = descriptorHeap->GetCPUDescriptorHandleForHeapStart();
			
			for (GTSL::uint32 index = 0; index < static_cast<GTSL::uint32>(bindingsUpdateInfos.ElementCount()); ++index) {
				auto& info = bindingsUpdateInfos[index];

				switch (info.Type)
				{
				case BindingType::SAMPLER: {
					for (auto e : info.BindingUpdateInfos) {
						D3D12_SAMPLER_DESC sampler_desc;
						renderDevice->GetID3D12Device2()->CreateSampler(&sampler_desc, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_HeapSize;
					}
					break;		
				}
				case BindingType::COMBINED_IMAGE_SAMPLER:
				case BindingType::SAMPLED_IMAGE:
				case BindingType::STORAGE_IMAGE:
				case BindingType::INPUT_ATTACHMENT: {
					for (auto e : info.BindingUpdateInfos) {
						renderDevice->GetID3D12Device2()->CreateShaderResourceView(e.TextureBindingUpdateInfo.Texture, nullptr, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_HeapSize;
					}

					break;
				}

				case BindingType::UNIFORM_TEXEL_BUFFER: GAL_DEBUG_BREAK;
				case BindingType::STORAGE_TEXEL_BUFFER: GAL_DEBUG_BREAK;

				case BindingType::UNIFORM_BUFFER:
				case BindingType::STORAGE_BUFFER:
				case BindingType::UNIFORM_BUFFER_DYNAMIC:
				case BindingType::STORAGE_BUFFER_DYNAMIC: {
					for (auto e : info.BindingUpdateInfos) {
						renderDevice->GetID3D12Device2()->CreateUnorderedAccessView(e.BufferBindingUpdateInfo.Buffer.GetID3D12Resource(), nullptr, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_HeapSize;
					}

					break;
				}

				case BindingType::ACCELERATION_STRUCTURE: {
					for (auto e : info.BindingUpdateInfos) {
						renderDevice->GetID3D12Device2()->CreateUnorderedAccessView(e.AccelerationStructureBindingUpdateInfo.AccelerationStructure.GetID3D12Resource(), nullptr, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_HeapSize;
					}

					break;
				}
				default: __debugbreak();
				}
			}
		}
	
	private:
		ID3D12DescriptorHeap* descriptorHeap = nullptr;
	};
}
