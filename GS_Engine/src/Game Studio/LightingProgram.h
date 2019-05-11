#pragma once

#include "Core.h"

#include "Program.h"

GS_CLASS LightingProgram :	public Program
{
public:
	LightingProgram();
	~LightingProgram();

	Uniform PositionTextureSampler;
	Uniform NormalTextureSampler;
	Uniform AlbedoTextureSampler;
};

