#pragma once

#include "GTSL/Core.h"
#include "GTSL/Range.h"

#include <d3d12.h>

#include "GAL/RenderCore.h"
#include "GTSL/Flags.h"

#if (_DEBUG)
#define DX_CHECK(func) if (func < 0) { __debugbreak(); }
#else
#define DX_CHECK(func) func;
#endif

namespace GAL
{
	template<typename T>
	void setName(T* handle, const GTSL::Range<const char8_t*> name) {
		if constexpr (_DEBUG) {
			if (name.ElementCount() != 0)
				handle->SetPrivateData(WKPDID_D3DDebugObjectName, name.ElementCount() - 1, name.begin());
		}
	}
	
	inline D3D12_COMMAND_LIST_TYPE ToDX12(const QueueType queueType) {
		if(queueType & QueueTypes::GRAPHICS) {
			return D3D12_COMMAND_LIST_TYPE_DIRECT;
		}
		
		if(queueType & QueueTypes::COMPUTE) {
			return D3D12_COMMAND_LIST_TYPE_COMPUTE;
		}
		
		if(queueType & QueueTypes::TRANSFER) {
			return D3D12_COMMAND_LIST_TYPE_COPY;
		}
	}
	
	inline D3D12_RESOURCE_STATES ToDX12(const BufferUse bufferUses) {
		GTSL::uint32 resourceStates = 0;
		TranslateMask<BufferUses::STORAGE, D3D12_RESOURCE_STATE_COMMON>(bufferUses, resourceStates);
		TranslateMask<BufferUses::TRANSFER_SOURCE, D3D12_RESOURCE_STATE_COPY_SOURCE>(bufferUses, resourceStates);
		TranslateMask<BufferUses::TRANSFER_DESTINATION, D3D12_RESOURCE_STATE_COPY_DEST>(bufferUses, resourceStates);
		return D3D12_RESOURCE_STATES(resourceStates);
	}
	
	inline D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE ToD3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE(const Operations operations) {
		switch (operations) {
		case Operations::UNDEFINED: return D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE_DISCARD;
		case Operations::DO: return D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE_PRESERVE;
		case Operations::CLEAR: return D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE_CLEAR;
		}
	}
	
	inline D3D12_RENDER_PASS_ENDING_ACCESS_TYPE ToD3D12_RENDER_PASS_ENDING_ACCESS_TYPE(const Operations operations) {
		switch (operations) {
		case Operations::UNDEFINED: return D3D12_RENDER_PASS_ENDING_ACCESS_TYPE_DISCARD;
		case Operations::DO: return D3D12_RENDER_PASS_ENDING_ACCESS_TYPE_PRESERVE;
		case Operations::CLEAR: return D3D12_RENDER_PASS_ENDING_ACCESS_TYPE_DISCARD;
		}
	}
	
	inline DXGI_FORMAT ToDX12(const ShaderDataType type) {
		switch (type) {
		case ShaderDataType::FLOAT: return DXGI_FORMAT_R32_FLOAT;
		case ShaderDataType::FLOAT2: return DXGI_FORMAT_R32G32_FLOAT;
		case ShaderDataType::FLOAT3: return DXGI_FORMAT_R32G32B32_FLOAT;
		case ShaderDataType::FLOAT4: return DXGI_FORMAT_R32G32B32A32_FLOAT;
		case ShaderDataType::INT:  return DXGI_FORMAT_R32_UINT;
		case ShaderDataType::INT2: return DXGI_FORMAT_R32G32_UINT;
		case ShaderDataType::INT3: return DXGI_FORMAT_R32G32B32_UINT;
		case ShaderDataType::INT4: return DXGI_FORMAT_R32G32B32A32_UINT;
		case ShaderDataType::BOOL: break;
		case ShaderDataType::MAT3: break;
		case ShaderDataType::MAT4: break;
		default: return DXGI_FORMAT_UNKNOWN;
		}
	}
		
	inline DXGI_FORMAT ToDX12(const IndexType indexType) {
		switch (indexType)
		{
		case IndexType::UINT8: return DXGI_FORMAT_R8_UINT;
		case IndexType::UINT16: return DXGI_FORMAT_R16_UINT;
		case IndexType::UINT32: return DXGI_FORMAT_R32_UINT;
		default: return DXGI_FORMAT_UNKNOWN;
		}
	}
	
	inline DXGI_FORMAT ToDX12(const Format format) {
		switch (format)
		{
		case Format::RGB_I8: return DXGI_FORMAT_UNKNOWN;
		case Format::RGBA_I8: return DXGI_FORMAT_R8G8B8A8_UNORM;
		case Format::RGBA_F16: return DXGI_FORMAT_R16G16B16A16_FLOAT;
		case Format::BGRA_I8: return DXGI_FORMAT_B8G8R8A8_UNORM;
		case Format::DEPTH32: return DXGI_FORMAT_D32_FLOAT;
		default: return DXGI_FORMAT_UNKNOWN;
		}
	}

	inline DXGI_FORMAT ToDX12(const FormatDescriptor format) {
		return ToDX12(MakeFormatFromFormatDescriptor(format));
	}

	inline D3D12_RESOURCE_DIMENSION ToDX12Type(const GTSL::Extent3D extent)
	{
		if (extent.Height != 1) {
			if (extent.Depth != 1) { return D3D12_RESOURCE_DIMENSION_TEXTURE3D; }
			return D3D12_RESOURCE_DIMENSION_TEXTURE2D;
		}

		return D3D12_RESOURCE_DIMENSION_TEXTURE1D;
	}

	inline D3D12_TEXTURE_LAYOUT ToDX12(const Tiling tiling) {
		switch (tiling) {
		case Tiling::OPTIMAL: return D3D12_TEXTURE_LAYOUT_UNKNOWN;
		case Tiling::LINEAR: return D3D12_TEXTURE_LAYOUT_ROW_MAJOR;
		}
	}
	
	inline D3D12_RESOURCE_STATES ToDX12(const TextureUse uses, const FormatDescriptor formatDescriptor) {
		GTSL::uint32 resourceStates = 0;

		if (uses & TextureUses::ATTACHMENT) {
			switch (formatDescriptor.Type)
			{
			case TextureType::COLOR: resourceStates |= D3D12_RESOURCE_STATE_RENDER_TARGET; break;
			case TextureType::DEPTH: resourceStates |= D3D12_RESOURCE_STATE_RENDER_TARGET; break;
			}
		}

		//TranslateMask<TextureUses::INPUT_ATTACHMENT, D3D12_RESOURCE_STATE_IN>(uses, resourceStates);
		//TranslateMask<TextureUses::SAMPLE, VK_IMAGE_USAGE_SAMPLED_BIT>(uses, resourceStates);
		//TranslateMask<TextureUses::STORAGE, VK_IMAGE_USAGE_STORAGE_BIT>(uses, resourceStates);
		TranslateMask<TextureUses::TRANSFER_DESTINATION, D3D12_RESOURCE_STATE_COPY_DEST>(uses, resourceStates);
		TranslateMask<TextureUses::TRANSFER_SOURCE, D3D12_RESOURCE_STATE_COPY_SOURCE>(uses, resourceStates);
		//TranslateMask<TextureUses::TRANSIENT_ATTACHMENT, VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT>(uses, resourceStates);

		return D3D12_RESOURCE_STATES(resourceStates);
	}

	inline D3D12_SHADER_VISIBILITY ToDX12(const ShaderStage shaderStage) {
		GTSL::UnderlyingType<D3D12_SHADER_VISIBILITY> shaderVisibility = 0;
		TranslateMask<ShaderStages::VERTEX, D3D12_SHADER_VISIBILITY::D3D12_SHADER_VISIBILITY_VERTEX>(shaderStage, shaderVisibility);
		TranslateMask<ShaderStages::FRAGMENT, D3D12_SHADER_VISIBILITY::D3D12_SHADER_VISIBILITY_PIXEL>(shaderStage, shaderVisibility);
		TranslateMask<ShaderStages::COMPUTE, D3D12_SHADER_VISIBILITY::D3D12_SHADER_VISIBILITY_DOMAIN> (shaderStage, shaderVisibility);
		return D3D12_SHADER_VISIBILITY(shaderVisibility);
	}
}