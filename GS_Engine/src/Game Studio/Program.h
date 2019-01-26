#pragma once

#include "Core.h"

#include "RendererObject.h"

#include "Uniform.h"

#include "Matrix4.h"

GS_CLASS Program : public RendererObject
{
public:
	Program(const char * VertexShaderPath, const char * FragmentShaderPath);
	~Program();

	void Bind() const override;

	void SetUniform(const Uniform & Uniform, float F1) const;
	void SetUniform(const Uniform & Uniform, float F1, float F2) const;
	void SetUniform(const Uniform & Uniform, float F1, float F2, float F3) const;
	void SetUniform(const Uniform & Uniform, float F1, float F2, float F3, float F4) const;
	void SetUniform(const Uniform & Uniform, int I1) const;
	void SetUniform(const Uniform & Uniform, int I1, int I2) const;
	void SetUniform(const Uniform & Uniform, int I1, int I2, int I3) const;
	void SetUniform(const Uniform & Uniform, int I1, int I2, int I3, int I4) const;
	void SetUniform(const Uniform & Uniform, bool B1) const;
	void SetUniform(const Uniform & Uniform, const Matrix4 & Matrix) const;
};