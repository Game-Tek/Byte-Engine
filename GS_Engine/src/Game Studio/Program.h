#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS Program : public RendererObject
{
public:
	Program();
	~Program();

	void Bind() const;

	void SetUniform(const char * UniformName, float F1) const;
	void SetUniform(const char * UniformName, float F1, float F2) const;
	void SetUniform(const char * UniformName, float F1, float F2, float F3) const;
	void SetUniform(const char * UniformName, float F1, float F2, float F3, float F4) const;
	void SetUniform(const char * UniformName, int I1) const;
	void SetUniform(const char * UniformName, int I1, int I2) const;
	void SetUniform(const char * UniformName, int I1, int I2, int I3) const;
	void SetUniform(const char * UniformName, int I1, int I2, int I3, int I4) const;
	void SetUniform(const char * UniformName, bool B1);
};

