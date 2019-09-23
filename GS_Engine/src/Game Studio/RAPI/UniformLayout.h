#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Containers/Array.hpp"

class RenderContext;

struct GS_API UniformSet
{
	uint8 UniformSetUniformsCount = 0;
	UniformType UniformSetType = UniformType::UNIFORM_BUFFER;
	ShaderType ShaderStage = ShaderType::ALL_STAGES;
	void* UniformData = nullptr;
};

struct GS_API UniformLayoutCreateInfo
{
	Array<UniformSet, MAX_DESCRIPTORS_PER_SET> PipelineUniformSets;
	RenderContext* RenderContext = nullptr;
};

struct GS_API UniformLayoutUpdateInfo
{
	Array<UniformSet, MAX_DESCRIPTORS_PER_SET> PipelineUniformSets;
};

class GS_API UniformLayout
{
};