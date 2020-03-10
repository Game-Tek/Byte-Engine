#pragma once

#include "Core.h"
#include "Containers/FString.h"
#include "Containers/Array.hpp"

#include "RenderCore.h"
#include "RenderMesh.h"

namespace RAPI
{

	struct StencilState
	{
		StencilCompareOperation FailOperation = StencilCompareOperation::ZERO;
		StencilCompareOperation PassOperation = StencilCompareOperation::ZERO;
		StencilCompareOperation DepthFailOperation = StencilCompareOperation::ZERO;
		CompareOperation CompareOperation = CompareOperation::NEVER;
	};

	struct StencilOperations
	{
		StencilState Front;
		StencilState Back;
	};

	struct ShaderInfo
	{
		ShaderType Type = ShaderType::VERTEX_SHADER;
		FString* ShaderCode = nullptr;
	};

	struct PipelineDescriptor
	{
		DArray<ShaderInfo> Stages = DArray<ShaderInfo>(8);
		CullMode CullMode = CullMode::CULL_NONE;
		bool DepthClampEnable = false;
		bool BlendEnable = false;
		BlendOperation ColorBlendOperation = BlendOperation::ADD;
		SampleCount RasterizationSamples = SampleCount::SAMPLE_COUNT_1;
		CompareOperation DepthCompareOperation = CompareOperation::NEVER;
		StencilOperations StencilOperations;
	};

	class RenderPass;

	class Pipeline : public RAPIObject
	{
	};

	class GraphicsPipeline : public Pipeline
	{
	public:
	};

	struct PushConstant
	{
		size_t Size = 0;
		ShaderType Stage = ShaderType::ALL_STAGES;
	};

	struct GraphicsPipelineCreateInfo : RenderInfo
	{
		RenderPass* RenderPass = nullptr;
		class Window* ActiveWindow = nullptr;
		VertexDescriptor* VDescriptor = nullptr;
		PipelineDescriptor PipelineDescriptor;
		GraphicsPipeline* ParentPipeline = nullptr;

		PushConstant* PushConstant = nullptr;
		Array<class BindingsSet*, 16> BindingsSets;
	};

}