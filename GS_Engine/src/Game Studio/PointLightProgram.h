#pragma once

#include "Core.h"

#include "Program.h"

GS_CLASS PointLightProgram : public Program
{
public:
	PointLightProgram();
	~PointLightProgram();

	Uniform AlbedoTextureSampler;
};

