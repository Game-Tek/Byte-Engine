#pragma once

#include "DX12.h"
#include "DX12AccelerationStructure.hpp"
#include "DX12Buffer.h"
#include "DX12Texture.h"
#include "GAL/Bindings.h"
#include "GAL/DX12/DX12RenderDevice.h"

namespace GAL {
	class DX12BindingsSet final {
	public:	
	private:
	};
	
	class DX12BindingsSetLayout final : BindingSetLayout {
	public:
		struct BindingDescriptor {
			BindingType BindingType;
			ShaderStage ShaderStage;
			GTSL::uint32 BindingsCount;
			BindingFlag Flags;
			GTSL::Range<const DX12Sampler*> Samplers;
		};

		DX12BindingsSetLayout(const DX12RenderDevice* renderDevice, GTSL::Range<const BindingDescriptor*> bindingsDescriptors) {
			GTSL::StaticVector<D3D12_ROOT_PARAMETER1, 16> parameters;
			GTSL::StaticVector<D3D12_STATIC_SAMPLER_DESC, 16> staticSamplers;

			for(uint32 i = 0; i < bindingsDescriptors.ElementCount(); ++i) {
				const auto& bd = bindingsDescriptors[i];

				auto& parameter = parameters.EmplaceBack();
				parameter.ShaderVisibility = ToDX12(bd.ShaderStage);

				if(!bd.Samplers.ElementCount()) {
					switch (bd.BindingType) {
					case BindingType::SAMPLER: break;
					case BindingType::COMBINED_IMAGE_SAMPLER: break;
					case BindingType::SAMPLED_IMAGE: {
						parameter.ParameterType = D3D12_ROOT_PARAMETER_TYPE_SRV;
						break;
					}
					case BindingType::STORAGE_IMAGE: {
						parameter.ParameterType = D3D12_ROOT_PARAMETER_TYPE_UAV;
						break;
					}
					case BindingType::UNIFORM_BUFFER: break;
					case BindingType::STORAGE_BUFFER: {
						parameter.ParameterType = D3D12_ROOT_PARAMETER_TYPE_CBV;
						parameter.Descriptor.Flags = D3D12_ROOT_DESCRIPTOR_FLAG_NONE;
						break;
					}
					case BindingType::INPUT_ATTACHMENT: break;
					case BindingType::ACCELERATION_STRUCTURE: break;
					}
				} else {
					auto& ss = staticSamplers.EmplaceBack();
					ss.ShaderVisibility = parameter.ShaderVisibility;
					ss.ShaderRegister = 0u; ss.RegisterSpace = 0u;
					ss.AddressU = D3D12_TEXTURE_ADDRESS_MODE_WRAP; ss.AddressV = D3D12_TEXTURE_ADDRESS_MODE_WRAP; ss.AddressW = D3D12_TEXTURE_ADDRESS_MODE_WRAP;
					ss.BorderColor = D3D12_STATIC_BORDER_COLOR_OPAQUE_BLACK;
					ss.ComparisonFunc = D3D12_COMPARISON_FUNC_GREATER_EQUAL;
					ss.Filter = D3D12_FILTER_MIN_MAG_MIP_LINEAR;
					ss.MaxAnisotropy = 8u;
					ss.MinLOD = 0u; ss.MaxLOD = 0u;
					ss.MipLODBias = 0.0f;
				}

			}

			{
				auto& pc = parameters.EmplaceBack();
				pc.ShaderVisibility = D3D12_SHADER_VISIBILITY_ALL;
				pc.ParameterType = D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS;
				pc.Constants.Num32BitValues = 128 / 4;
				pc.Constants.RegisterSpace = 0u; pc.Constants.ShaderRegister = 0u;
			}

			D3D12_VERSIONED_ROOT_SIGNATURE_DESC rootSignatureDesc{ D3D_ROOT_SIGNATURE_VERSION_1_1 };
			rootSignatureDesc.Desc_1_1.Flags = D3D12_ROOT_SIGNATURE_FLAG_LOCAL_ROOT_SIGNATURE | D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS;
			rootSignatureDesc.Desc_1_1.NumParameters = parameters.GetLength();
			rootSignatureDesc.Desc_1_1.pParameters = parameters.GetData();
			rootSignatureDesc.Desc_1_1.NumStaticSamplers = staticSamplers.GetLength();
			rootSignatureDesc.Desc_1_1.pStaticSamplers = staticSamplers.GetData();

			ID3DBlob* rootSignatureBlob, * errorBlob;

			D3D12SerializeVersionedRootSignature(&rootSignatureDesc, &rootSignatureBlob, &errorBlob);

			renderDevice->GetID3D12Device2()->CreateRootSignature(0, rootSignatureBlob->GetBufferPointer(), rootSignatureBlob->GetBufferSize(), __uuidof(ID3D12RootSignature), reinterpret_cast<void**>(&rootSignature));
		}

	private:
		ID3D12RootSignature* rootSignature = nullptr;
	};

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
			DX12Texture Texture;
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
		void Update(const DX12RenderDevice* renderDevice, const DX12BindingsSet* bindingsSet, GTSL::Range<const BindingsUpdateInfo*> bindingsUpdateInfos, const ALLOCATOR& allocator) {
			auto sbv_srv_uav_size = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
			auto rtvHeapSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);
			auto dsvHeapSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_DSV);
			auto samplerHandleSize = renderDevice->GetID3D12Device2()->GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER);

			auto sbv_srv_uav_handle = descriptorHeapCBV_SRV_UAV->GetCPUDescriptorHandleForHeapStart();

			for (GTSL::uint32 index = 0; index < static_cast<GTSL::uint32>(bindingsUpdateInfos.ElementCount()); ++index) {
				auto& info = bindingsUpdateInfos[index];

				switch (info.Type) {
				case BindingType::SAMPLER: {
					for (auto e : info.BindingUpdateInfos) {
						sbv_srv_uav_handle.ptr += sbv_srv_uav_size;
					}
					break;		
				}
				case BindingType::COMBINED_IMAGE_SAMPLER:
				case BindingType::SAMPLED_IMAGE:
				case BindingType::STORAGE_IMAGE:
				case BindingType::INPUT_ATTACHMENT: {
					for (auto e : info.BindingUpdateInfos) {
						D3D12_SHADER_RESOURCE_VIEW_DESC resourceViewDesc;
						resourceViewDesc.Texture2D.MipLevels = 1;
						resourceViewDesc.Texture2D.MostDetailedMip = 0u;
						resourceViewDesc.Texture2D.PlaneSlice = 0u;
						resourceViewDesc.Texture2D.ResourceMinLODClamp = 0.0f;
						renderDevice->GetID3D12Device2()->CreateShaderResourceView(e.TextureBindingUpdateInfo.Texture.GetID3D12Resource(), &resourceViewDesc, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_size;
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
						sbv_srv_uav_handle.ptr += sbv_srv_uav_size;
					}

					break;
				}
				case BindingType::ACCELERATION_STRUCTURE: {
					for (auto e : info.BindingUpdateInfos) {
						D3D12_SHADER_RESOURCE_VIEW_DESC resourceViewDesc;
						resourceViewDesc.RaytracingAccelerationStructure.Location;
						renderDevice->GetID3D12Device2()->CreateUnorderedAccessView(e.AccelerationStructureBindingUpdateInfo.AccelerationStructure.GetID3D12Resource(), nullptr, sbv_srv_uav_handle);
						sbv_srv_uav_handle.ptr += sbv_srv_uav_size;
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
