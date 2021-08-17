#pragma once

#include "DX12.h"
#include "DX12RenderDevice.h"
#include "GAL/Pipelines.h"
#include "GAL/RenderCore.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/Extent.h>

#include "GTSL/Vector.hpp"

namespace GAL
{
	class DX12Buffer;
	class DX12Sampler;
	class DX12TextureView;
	
	struct DX12PipelineDescriptor
	{
		CullMode CullMode = CullMode::CULL_NONE;
		bool DepthClampEnable = false;
		bool BlendEnable = false;
		BlendOperation ColorBlendOperation = BlendOperation::ADD;
		SampleCount RasterizationSamples = SampleCount::SAMPLE_COUNT_1;
		bool DepthTest = false;
		bool DepthWrite = false;
		CompareOperation DepthCompareOperation = CompareOperation::NEVER;
		bool StencilTest = false;
		StencilOperations StencilOperations;
	};

	struct DX12ShaderInfo
	{
		ShaderType Type;
		const class DX12Shader* Shader = nullptr;
		GTSL::Range<const GTSL::byte*> ShaderData;
	};

	class DX12PipelineLayout final
	{
	public:
		DX12PipelineLayout() = default;

		struct BindingDescriptor
		{
			BindingType BindingType;
			ShaderStage ShaderStage;
			GTSL::uint32 UniformCount = 0;
			BindingFlag Flags;
		};

		struct ImageBindingDescriptor : BindingDescriptor
		{
			GTSL::Range<const DX12TextureView*> ImageViews;
			GTSL::Range<const DX12Sampler*> Samplers;
			GTSL::Range<const TextureLayout*> Layouts;
		};

		struct BufferBindingDescriptor : BindingDescriptor
		{
			GTSL::Range<const DX12Buffer*> Buffers;
			GTSL::Range<const GTSL::uint32*> Offsets;
			GTSL::Range<const GTSL::uint32*> Sizes;
		};

		void Initialize(const CreateInfo& info) {
			GTSL::StaticVector<D3D12_ROOT_PARAMETER, 12> rootParameters;

			if (info.PushConstant) {
				auto& pushConstant = rootParameters.EmplaceBack();
				pushConstant.ParameterType = D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS;
				pushConstant.ShaderVisibility = ToDX12(info.PushConstant->Stage);
				pushConstant.Constants.Num32BitValues = info.PushConstant->NumberOf4ByteSlots;
				pushConstant.Constants.RegisterSpace = 0;
				pushConstant.Constants.ShaderRegister = 0;
			}
			
			for (GTSL::uint32 i = 0; i < info.BindingsDescriptors.ElementCount(); ++i)
			{
				D3D12_ROOT_PARAMETER rootParameter;
				rootParameter.ParameterType;
				rootParameter.ShaderVisibility;
				rootParameter.Constants;
				rootParameter.Descriptor;
				rootParameter.DescriptorTable;
				rootParameters.EmplaceBack(rootParameter);
			}

			D3D12_ROOT_SIGNATURE_DESC rootSignatureDesc;
			rootSignatureDesc.Flags = D3D12_ROOT_SIGNATURE_FLAG_NONE;
			rootSignatureDesc.NumParameters = rootParameters.GetLength();
			rootSignatureDesc.pParameters = rootParameters.begin();
			rootSignatureDesc.NumStaticSamplers = 0;
			rootSignatureDesc.pStaticSamplers = nullptr;
			DX_CHECK(D3D12SerializeRootSignature(&rootSignatureDesc, D3D_ROOT_SIGNATURE_VERSION_1_1, nullptr, nullptr));
			DX_CHECK(info.RenderDevice->GetID3D12Device2()->CreateRootSignature(0, nullptr, 0, __uuidof(ID3D12RootSignature), nullptr));
			setName(rootSignature, {});
		}

		void Destroy(const DX12RenderDevice* renderDevice) {
			rootSignature->Release();
			debugClear(rootSignature);
		}
		
		~DX12PipelineLayout() = default;
		
	private:
		ID3D12RootSignature* rootSignature = nullptr;
	};

	class DX12Pipeline : public Pipeline {
	public:
		void Destroy(const DX12RenderDevice* renderDevice) {
			pipelineState->Release();
			debugClear(pipelineState);
		}

		[[nodiscard]] ID3D12PipelineState* GetID3D12PipelineState() const { return pipelineState; }

	protected:
		ID3D12PipelineState* pipelineState = nullptr;
	};
	
	class DX12RasterPipeline final : public DX12Pipeline
	{
	public:
		DX12RasterPipeline() = default;

		struct CreateInfo final : DX12CreateInfo
		{
			const class VulkanRenderPass* RenderPass = nullptr;
			GTSL::Extent2D SurfaceExtent;
			GTSL::Range<const VertexElement*> VertexDescriptor;
			DX12PipelineDescriptor PipelineDescriptor;
			GTSL::Range<const DX12ShaderInfo*> Stages;
			bool IsInheritable = false;
			const DX12PipelineLayout* PipelineLayout = nullptr;
			const DX12RasterPipeline* ParentPipeline = nullptr;
			const class DX12PipelineCache* PipelineCache = nullptr;
			GTSL::uint32 SubPass = 0;
		};
		void Initialize(const CreateInfo& info) {
			GTSL::Buffer<GTSL::StaticAllocator<1024>> buffer(1024, 16);
			//buffer.Allocate(1024, 8, allocator);

			GTSL::StaticVector<D3D12_INPUT_ELEMENT_DESC, 12> vertexElements;

			D3D12_PIPELINE_STATE_SUBOBJECT_TYPE type;

			{
				{
					for (GTSL::uint32 i = 0; i < info.Stages.ElementCount(); ++i)
					{
						switch (info.Stages[i].Type)
						{
						case ShaderType::VERTEX: type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_VS; break;
						case ShaderType::TESSELLATION_CONTROL: type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_VS; break;
						case ShaderType::TESSELLATION_EVALUATION: type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_VS; break;
						case ShaderType::GEOMETRY: type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_GS; break;
						case ShaderType::FRAGMENT: type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS; break;
						case ShaderType::COMPUTE: type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_CS; break;
						default: type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MAX_VALID;
						}

						buffer.CopyBytes(sizeof(D3D12_PIPELINE_STATE_SUBOBJECT_TYPE), reinterpret_cast<const byte*>(&type));
						D3D12_SHADER_BYTECODE bytecode;
						bytecode.BytecodeLength = info.Stages[i].ShaderData.ElementCount();
						bytecode.pShaderBytecode = info.Stages[i].ShaderData.begin();
						buffer.CopyBytes(sizeof(D3D12_SHADER_BYTECODE), reinterpret_cast<const byte*>(&bytecode));
					}
				}

				{
					type = D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_INPUT_LAYOUT;
					buffer.CopyBytes(sizeof(D3D12_PIPELINE_STATE_SUBOBJECT_TYPE), reinterpret_cast<const byte*>(&type));

					D3D12_INPUT_LAYOUT_DESC inputLayoutDesc;
					inputLayoutDesc.NumElements = static_cast<UINT32>(info.VertexDescriptor.ElementCount());

					GTSL::uint32 offset = 0;

					for (GTSL::uint32 i = 0; i < inputLayoutDesc.NumElements; ++i)
					{

						D3D12_INPUT_ELEMENT_DESC elementDesc;
						elementDesc.Format = ToDX12(info.VertexDescriptor[i].Type);
						elementDesc.AlignedByteOffset = offset;
						elementDesc.SemanticIndex = 0;
						elementDesc.InputSlot = i;
						elementDesc.InputSlotClass = D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA;
						elementDesc.InstanceDataStepRate = 0;

						offset += ShaderDataTypesSize(info.VertexDescriptor[i].Type);

						elementDesc.SemanticName = info.VertexDescriptor[i].Identifier.begin();
						vertexElements.EmplaceBack(elementDesc);
					}

					inputLayoutDesc.pInputElementDescs = vertexElements.begin();
					buffer.CopyBytes(sizeof(D3D12_INPUT_LAYOUT_DESC), reinterpret_cast<const byte*>(&inputLayoutDesc));
				}
			}

			D3D12_PIPELINE_STATE_STREAM_DESC pipelineStateStream;
			pipelineStateStream.SizeInBytes = buffer.GetLength();
			pipelineStateStream.pPipelineStateSubobjectStream = buffer.GetData();

			info.RenderDevice->GetID3D12Device2()->CreatePipelineState(&pipelineStateStream, __uuidof(ID3D12PipelineState), reinterpret_cast<void**>(&pipelineState));
			setName(pipelineState, info.Name);
		}
		
		~DX12RasterPipeline() = default;
		
	private:

	};
}
