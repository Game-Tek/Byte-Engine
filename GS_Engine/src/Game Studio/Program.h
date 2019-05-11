#pragma once

#include "Core.h"

#include "RendererObject.h"

#include "Uniform.h"

GS_CLASS Program : public RendererObject
{
public:
	Program(const char * VertexShaderPath, const char * FragmentShaderPath);
	~Program();

	void Bind() const override;

	Uniform ModelMatrix;
	Uniform ViewMatrix;
	Uniform ProjectionMatrix;
};