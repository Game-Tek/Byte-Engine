#pragma once

#include "Core.h"
#include "Containers/FString.h"

#include "RenderCore.h"
#include "Mesh.h"


GS_STRUCT StencilState
{
	StencilCompareOperation FailOperation = StencilCompareOperation::ZERO;
	StencilCompareOperation PassOperation = StencilCompareOperation::ZERO;
	StencilCompareOperation DepthFailOperation = StencilCompareOperation::ZERO;
	CompareOperation CompareOperation = CompareOperation::NEVER;
};

GS_STRUCT StencilOperations
{
	StencilState Front;
	StencilState Back;
};

GS_STRUCT ShaderInfo
{
	ShaderType Type = ShaderType::VERTEX_SHADER;
	FString ShaderCode = FString("NO CODE");
};

GS_STRUCT ShaderStages
{
	ShaderInfo* VertexShader = nullptr;
	ShaderInfo* TessellationControlShader = nullptr;
	ShaderInfo* TessellationEvaluationShader = nullptr;
	ShaderInfo* GeometryShader = nullptr;
	ShaderInfo* FragmentShader = nullptr;
};

GS_STRUCT PipelineDescriptor
{
	ShaderStages Stages;
	CullMode CullMode = CullMode::CULL_NONE;
	bool DepthClampEnable = false;
	bool BlendEnable = false;
	BlendOperation ColorBlendOperation = BlendOperation::ADD;
	SampleCount RasterizationSamples = SampleCount::SAMPLE_COUNT_1;
	CompareOperation DepthCompareOperation = CompareOperation::NEVER;
	StencilOperations StencilOperations;
};

class RenderPass;
class UniformLayout;

GS_CLASS GraphicsPipeline
{
public:
};

GS_STRUCT GraphicsPipelineCreateInfo
{
	Extent2D SwapchainSize = {1280, 720 };
	RenderPass* RenderPass = nullptr;
	VertexDescriptor* VDescriptor = nullptr;
	PipelineDescriptor PipelineDescriptor;
	UniformLayout* UniformLayout = nullptr;
	GraphicsPipeline* ParentPipeline = nullptr;
};