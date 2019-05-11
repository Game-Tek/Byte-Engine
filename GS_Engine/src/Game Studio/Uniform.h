#pragma once

#include "Core.h"

#include "RendererObject.h"

#include "Vector2.h"
#include "Vector3.h"
#include "Matrix4.h"

class Program;

//Used to represent a shader language uniform from the C++ side.
GS_CLASS Uniform : public RendererObject
{
public:
	Uniform() = default;
	Uniform(const Program & Program, const char * UniformName);
	Uniform(Program * Program, const char * UniformName);
	~Uniform();

	void Setup(Program* Program, const char* UniformName);

	void Set(float Other) const;
	void Set(const Vector2 & Other) const;
	void Set(const Vector3 & Other) const;
	void Set(const Vector4 & Other) const;
	void Set(int32 Other) const;
	void Set(bool Other) const;
	void Set(const Matrix4 & Other) const;
	void Set(Matrix4 * Other) const;
};

