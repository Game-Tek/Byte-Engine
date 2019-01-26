#pragma once

#include "RendererObject.h"

class Uniform : public RendererObject
{
public:
	Uniform(const RendererObject & Program, const char * UniformName);
	~Uniform();
};

