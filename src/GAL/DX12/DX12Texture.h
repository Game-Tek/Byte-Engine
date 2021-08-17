#pragma once

#include "GAL/RenderCore.h"

#include "DX12.h"
#include <GTSL/Extent.h>
#include "DX12Memory.h"
#include "DX12RenderDevice.h"
#include "GAL/Texture.h"

namespace GAL
{
	class DX12Texture final : public Texture
	{	
	public:
		DX12Texture() = default;

		void GetMemoryRequirements(const DX12RenderDevice* renderDevice, MemoryRequirements* memoryRequirements, TextureLayout initialLayout, TextureUse uses,
			FormatDescriptor format, const GTSL::Extent3D extent, const Tiling tiling, GTSL::uint8 mipLevels) {
			D3D12_RESOURCE_DESC resourceDesc;
			resourceDesc.Width = extent.Width; resourceDesc.Height = extent.Height; resourceDesc.DepthOrArraySize = extent.Depth;
			resourceDesc.Dimension = ToDX12Type(extent);
			resourceDesc.Layout = ToDX12(tiling);
			resourceDesc.Format = ToDX12(MakeFormatFromFormatDescriptor(format));
			resourceDesc.MipLevels = mipLevels;
			resourceDesc.Flags = D3D12_RESOURCE_FLAG_NONE;

			const auto allocInfo = renderDevice->GetID3D12Device2()->GetResourceAllocationInfo(0, 1, &resourceDesc);

			memoryRequirements->Alignment = static_cast<GTSL::uint32>(allocInfo.Alignment);
			memoryRequirements->Size = static_cast<GTSL::uint32>(allocInfo.SizeInBytes);
			memoryRequirements->MemoryTypes = 0;
		}

		void Initialize(const DX12RenderDevice* renderDevice, const MemoryRequirements& memoryRequirements,
		                const DX12Memory deviceMemory, const GTSL::Extent3D extent, const TextureUse uses,
		                const FormatDescriptor format, const Tiling tiling, const GTSL::uint32 offset) {
			D3D12_RESOURCE_DESC resourceDesc;
			resourceDesc.Width = extent.Width;
			resourceDesc.Height = extent.Height;
			resourceDesc.DepthOrArraySize = extent.Depth;
			resourceDesc.Dimension = ToDX12Type(extent);
			resourceDesc.Layout = ToDX12(tiling);
			resourceDesc.Format = ToDX12(MakeFormatFromFormatDescriptor(format));
			resourceDesc.Alignment = memoryRequirements.Alignment;
			resourceDesc.Flags = D3D12_RESOURCE_FLAG_NONE;

			resourceDesc.SampleDesc.Count = 1;
			resourceDesc.SampleDesc.Quality = 0;

			DX_CHECK(renderDevice->GetID3D12Device2()->CreatePlacedResource(deviceMemory.GetID3D12Heap(), offset, &
				resourceDesc, ToDX12(uses, format), nullptr, __uuidof(ID3D12Resource), reinterpret_cast<void**>(&resource)));
			//setName(resource, info);
		}
		
		void Destroy(const DX12RenderDevice* renderDevice) {
			resource->Release();
			debugClear(resource);
		}

		[[nodiscard]] ID3D12Resource* GetID3D12Resource() const { return resource; }
		
		~DX12Texture() = default;
		
	private:
		ID3D12Resource* resource = nullptr;
	};

	class DX12TextureView final
	{
	public:
		DX12TextureView() = default;

		void Initialize(const DX12RenderDevice* renderDevice, const GTSL::Range<const char8_t*> name, const DX12Texture texture, const FormatDescriptor formatDescriptor, const GTSL::Extent3D extent, const GTSL::uint8 mipLevels)
		{
			D3D12_CPU_DESCRIPTOR_HANDLE cpu_descriptor_handle;
			
			D3D12_UNORDERED_ACCESS_VIEW_DESC unordered_access_view_desc;
			unordered_access_view_desc.Format = ToDX12(MakeFormatFromFormatDescriptor(formatDescriptor));
			unordered_access_view_desc.ViewDimension = D3D12_UAV_DIMENSION_TEXTURE2D;
			unordered_access_view_desc.Texture2D.MipSlice = 0;
			unordered_access_view_desc.Texture2D.PlaneSlice = 0;
			renderDevice->GetID3D12Device2()->CreateUnorderedAccessView(nullptr, nullptr, &unordered_access_view_desc, cpu_descriptor_handle);

			D3D12_SHADER_RESOURCE_VIEW_DESC shader_resource_view_desc;
			shader_resource_view_desc.Format = ToDX12(MakeFormatFromFormatDescriptor(formatDescriptor));
			shader_resource_view_desc.ViewDimension = D3D12_SRV_DIMENSION_TEXTURE2D;
			shader_resource_view_desc.Texture2D.PlaneSlice = 0;
			shader_resource_view_desc.Texture2D.MipLevels = 1;
			shader_resource_view_desc.Texture2D.MostDetailedMip = 0;
			shader_resource_view_desc.Texture2D.ResourceMinLODClamp = 0.0f;
			
			renderDevice->GetID3D12Device2()->CreateShaderResourceView(texture.GetID3D12Resource(), &shader_resource_view_desc, );
		}
	
	private:
		ID3D12Resource* tex_2d = nullptr;
	};

	class DX12Sampler final
	{
	public:
		DX12Sampler() = default;

		void Initialize(const DX12RenderDevice* renderDevice, const GTSL::uint8 anisotropy) {
			D3D12_SAMPLER_DESC samplerDesc;
			samplerDesc.MaxAnisotropy = anisotropy;
			samplerDesc.AddressU = D3D12_TEXTURE_ADDRESS_MODE_CLAMP;
			samplerDesc.AddressV = D3D12_TEXTURE_ADDRESS_MODE_CLAMP;
			samplerDesc.AddressW = D3D12_TEXTURE_ADDRESS_MODE_CLAMP;
			samplerDesc.BorderColor[0] = 0.0f;
			samplerDesc.BorderColor[1] = 0.0f;
			samplerDesc.BorderColor[2] = 0.0f;
			samplerDesc.BorderColor[3] = 0.0f;
			samplerDesc.ComparisonFunc = D3D12_COMPARISON_FUNC_ALWAYS;
			samplerDesc.Filter = D3D12_FILTER_ANISOTROPIC;
			samplerDesc.MaxLOD = 0.0f;
			samplerDesc.MinLOD = 0.0f;
			samplerDesc.MipLODBias = 0.0f;

			renderDevice->GetID3D12Device2()->CreateSampler(&samplerDesc, sampler);
		}
		
	private:
		D3D12_CPU_DESCRIPTOR_HANDLE sampler;
	};
}
