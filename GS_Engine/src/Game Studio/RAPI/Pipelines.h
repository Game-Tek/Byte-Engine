#pragma once

#include "Core.h"
#include "Containers/FString.h"

#include "RenderCore.h"


GS_STRUCT ShaderInfo
{
	ShaderType Type;
	FString ShaderCode;
};

GS_STRUCT ShaderStages
{
	ShaderInfo* VertexShader		= nullptr;
	ShaderInfo* TessellationShader	= nullptr;
	ShaderInfo* GeometryShader		= nullptr;
	ShaderInfo* FragmentShader		= nullptr;
};

class RenderPass;

GS_STRUCT GraphicsPipelineCreateInfo
{
	ShaderStages Stages;
	Extent2D SwapchainSize;
	RenderPass* RenderPass = nullptr;
};

GS_CLASS GraphicsPipeline
{
public:
};

GS_STRUCT ComputePipelineCreateInfo
{

};

GS_CLASS ComputePipeline
{
public:
};