#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Containers/Array.hpp"

class RenderContext;

GS_STRUCT UniformSet
{
	uint8 UniformSetUniformsCount = 0;
	UniformType UniformSetType = UniformType::UNIFORM_BUFFER;
	void* UniformData = nullptr;
	ShaderType ShaderStage = ShaderType::ALL_STAGES;
};

GS_STRUCT UniformLayoutCreateInfo
{
	Array<UniformSet, MAX_DESCRIPTORS_PER_SET> PipelineUniformSets;
	RenderContext* RenderContext = nullptr;
};

GS_CLASS UniformLayout
{
};