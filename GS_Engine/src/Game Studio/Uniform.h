#pragma once

#include "RendererObject.h"

#include "Program.h"

class Uniform : public RendererObject
{
public:
	Uniform(const Program & Progr, const char * UniformName);
	~Uniform();
};

