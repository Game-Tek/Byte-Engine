#pragma once

#include "Core.h"

#include "Program.h"

#include "Uniform.h"

GS_CLASS GBufferProgram : public Program
{
public:
	GBufferProgram();
	~GBufferProgram();

	Uniform ViewMatrix;
	Uniform ProjMatrix;
	Uniform ModelMatrix;
};

