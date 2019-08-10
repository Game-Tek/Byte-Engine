#pragma once

#include "Core.h"
#include "Containers/FString.h"

#include "RenderCore.h"
#include "Mesh.h"


GS_STRUCT ShaderInfo
{
	ShaderType Type = ShaderType::VERTEX_SHADER;
	FString ShaderCode = FString("NO CODE");
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
	Extent2D SwapchainSize = {1280, 720 };
	RenderPass* RenderPass = nullptr;
	VertexDescriptor* VDescriptor = nullptr;
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