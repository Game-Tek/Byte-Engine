#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Containers/Array.hpp"

GS_STRUCT UniformSet
{
	uint8 UniformSetUniformsCount = 0;
	UniformType UniformSetType;
	ShaderType ShaderStage;
};

GS_STRUCT PipelineLayoutCreateInfo
{
	Array<UniformSet, 8> PipelineUniformSets;
};

GS_CLASS PipelineLayout
{
};