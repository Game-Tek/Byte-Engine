#pragma once

#include "Core.h"
#include "Containers/FString.h"

#include "RenderCore.h"
#include "Mesh.h"
#include "Containers/Array.hpp"


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
	char* ShaderCode = nullptr;
};

struct GS_API PipelineDescriptor
{
	Array<ShaderInfo, 8> Stages;
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

struct GS_API GraphicsPipelineCreateInfo
{
	RenderPass* RenderPass = nullptr;
	VertexDescriptor* VDescriptor = nullptr;
	PipelineDescriptor PipelineDescriptor;
	UniformLayout* UniformLayout = nullptr;
	GraphicsPipeline* ParentPipeline = nullptr;
};