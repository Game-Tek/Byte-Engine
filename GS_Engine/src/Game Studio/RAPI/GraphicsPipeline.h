#pragma once

#include "Core.h"
#include "Containers/FString.h"

#include "RenderCore.h"
#include "RenderMesh.h"


struct GS_API StencilState
{
	StencilCompareOperation FailOperation = StencilCompareOperation::ZERO;
	StencilCompareOperation PassOperation = StencilCompareOperation::ZERO;
	StencilCompareOperation DepthFailOperation = StencilCompareOperation::ZERO;
	CompareOperation CompareOperation = CompareOperation::NEVER;
};

struct GS_API StencilOperations
{
	StencilState Front;
	StencilState Back;
};

struct GS_API ShaderInfo
{
	ShaderType Type = ShaderType::VERTEX_SHADER;
	FString* ShaderCode = nullptr;
};

struct GS_API PipelineDescriptor
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
class UniformLayout;

class GS_API GraphicsPipeline
{
public:
};

class Window;

struct GS_API GraphicsPipelineCreateInfo
{
	RenderPass* RenderPass = nullptr;
	Window* ActiveWindow = nullptr;
	VertexDescriptor* VDescriptor = nullptr;
	PipelineDescriptor PipelineDescriptor;
	UniformLayout* UniformLayout = nullptr;
	GraphicsPipeline* ParentPipeline = nullptr;
};