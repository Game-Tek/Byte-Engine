#pragma once

#include "Core.h"

#define MAX_SHADER_STAGES 6

#include "Shader.h"

GS_STRUCT StageInfo
{
	Shader* Shader[MAX_SHADER_STAGES];
	uint8 ShaderCount = 2;
};

GS_STRUCT GraphicsPipelineCreateInfo
{
	StageInfo StagesInfo;
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