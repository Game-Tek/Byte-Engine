#pragma once

#include "DX12.h"
#include "DX12AccelerationStructure.hpp"
#include "DX12Buffer.h"
#include "DX12Texture.h"
#include "GAL/Bindings.h"
#include "GAL/DX12/DX12RenderDevice.h"

namespace GAL
{
	class DX12BindingsSet final {
	public:
		void Initialize(const DX12RenderDevice* renderDevice) {
			D3D12_ROOT_CONSTANTS rootConstants;
			rootConstants.ShaderRegister = 0;
			rootConstants.RegisterSpace = 0;
			rootConstants.Num32BitValues = 0; //
			renderDevice->GetID3D12Device2()->CreateRootSignature(0, nullptr, 0ull, __uuidof(ID3D12RootSignature), reinterpret_cast<void**>(&rootSignature));
		}
	
	private:
		ID3D12RootSignature* rootSignature = nullptr;
	};
	
	class DX12BindingsSetLayout final
	{};

	class DX12BindingsPool final : BindingsPool {
	public:
		void Initialize(const DX12RenderDevice* renderDevice, GTSL::Range<const BindingsPoolSize*> bindingsPoolSizes, GTSL::uint32 maxSets) {
			D3D12_DESCRIPTOR_HEAP_DESC descHeapCbvSrv = {}, descHeapSampler = {}, descHeapRTV = {}, descHeapDSV = {};
			
			for (auto& e : bindingsPoolSizes) {
				switch(e.BindingType) {
				case BindingType::INPUT_ATTACHMENT: {
					descHeapRTV.NumDescriptors += e.Count;
					descHeapRTV.Flags = D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE;
					descHeapRTV.Type = D3D12_DESCRIPTOR_HEAP_TYPE_RTV;
					descHeapRTV.NodeMask;
					break;
				}
				case BindingType::UNIFORM_BUFFER:
				case BindingType::STORAGE_BUFFER:
				case BindingType::SAMPLED_IMAGE: {
					descHeapSampler.NumDescriptors += e.Count;
					descHeapSampler.Flags = D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE;
					descHeapSampler.Type = D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV;
					descHeapSampler.NodeMask;
					break;
				}
				case BindingType::SAMPLER: {
					descHeapSampler.NumDescriptors += e.Count;
					descHeapSampler.Flags = D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE;
					descHeapSampler.Type = D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER;
					descHeapSampler.NodeMask;
					break;
				}
				}
			}

			renderDevice->GetID3D12Device2()->CreateDescriptorHeap(&descHeapCbvSrv, __uuidof(ID3D12DescriptorHeap), reinterpret_cast<void**>(&descriptorHeapCBV_SRV_UAV));
			renderDevice->GetID3D12Device2()->CreateDescriptorHeap(&descHeapSampler, __uuidof(ID3D12DescriptorHeap), reinterpret_cast<void**>(&samplerDescriptorHeap));
			renderDevice->GetID3D12Device2()->CreateDescriptorHeap(&descHeapRTV, __uuidof(ID3D12DescriptorHeap), reinterpret_cast<void**>(&rtvDescriptorHeap));
			renderDevice->GetID3D12Device2()->CreateDescriptorHeap(&descHeapDSV, __uuidof(ID3D12DescriptorHeap), reinterpret_cast<void**>(&dsvDescriptorHeap));
		}

		//ID3D12DescriptorHeap* GetID3D12DescriptorHeap() const { return descriptorHeap; }

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
			auto sbv_srv_uav_size = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
			auto rtvHeapSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);
			auto dsvHeapSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_DSV);
			auto samplerHandleSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER);

			auto sbv_srv_uav_handle = descriptorHeap->GetCPUDescriptorHandleForHeapStart();

			for (GTSL::uint32 index = 0; index < static_cast<GTSL::uint32>(bindingsUpdateInfos.ElementCount()); ++index) {
				auto& info = bindingsUpdateInfos[index];

				switch (info.Type) {
				case BindingType::SAMPLER: {
					for (auto e : info.BindingUpdateInfos) {
						sbv_srv_uav_handle.ptr += sbv_srv_uav_HeapSize;
					}
					break;		
				}
				case BindingType::COMBINED_IMAGE_SAMPLER:
				case BindingType::SAMPLED_IMAGE:
				case BindingType::STORAGE_IMAGE:
				case BindingType::INPUT_ATTACHMENT: {
					for (auto e : info.BindingUpdateInfos) {
						D3D12_SHADER_RESOURCE_VIEW_DESC resourceViewDesc;
						resourceViewDesc.Texture2D.MipLevels;
						resourceViewDesc.Texture2D.MostDetailedMip;
						resourceViewDesc.Texture2D.PlaneSlice;
						resourceViewDesc.Texture2D.ResourceMinLODClamp;
						renderDevice->GetID3D12Device2()->CreateShaderResourceView(e.TextureBindingUpdateInfo.Texture.GetID3D12Resource(), &resourceViewDesc, sbv_srv_uav_handle);
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
						D3D12_SHADER_RESOURCE_VIEW_DESC resourceViewDesc;
						resourceViewDesc.Buffer.FirstElement;
						resourceViewDesc.Buffer.Flags;
						resourceViewDesc.Buffer.NumElements;
						resourceViewDesc.Buffer.StructureByteStride;
						renderDevice->GetID3D12Device2()->CreateUnorderedAccessView(e.BufferBindingUpdateInfo.Buffer.GetID3D12Resource(), &resourceViewDesc, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_HeapSize;
					}

					break;
				}
				case BindingType::ACCELERATION_STRUCTURE: {
					for (auto e : info.BindingUpdateInfos) {
						D3D12_SHADER_RESOURCE_VIEW_DESC resourceViewDesc;
						resourceViewDesc.RaytracingAccelerationStructure.Location;
						renderDevice->GetID3D12Device2()->CreateUnorderedAccessView(e.AccelerationStructureBindingUpdateInfo.AccelerationStructure.GetID3D12Resource(), nullptr, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_HeapSize;
					}

					break;
				}
				}
			}
		}
	
	private:
		ID3D12DescriptorHeap* descriptorHeapCBV_SRV_UAV = nullptr, *samplerDescriptorHeap = nullptr, *rtvDescriptorHeap = nullptr, *dsvDescriptorHeap = nullptr;
	};
}
