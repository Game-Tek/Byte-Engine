#pragma once

#include "Core.h"

#include "RendererObject.h"

#include "Matrix4.h"

GS_CLASS Program : public RendererObject
{
public:
	Program(const char * VertexShaderPath, const char * FragmentShaderPath);
	~Program();

	void Bind() const override;
};