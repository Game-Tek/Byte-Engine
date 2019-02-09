#pragma once

#include "Core.h"

#include <GLAD/glad.h>

#include "GL.h"

#include "Program.h"

#include "Vector2.h"
#include "Vector3.h"
#include "Matrix4.h"

//Used to represent a shader language uniform from the C++ side.
GS_CLASS Uniform : public RendererObject
{
public:
	Uniform(Program * Program, const char * UniformName);
	~Uniform();

	void operator=(float Other) const
	{
		GS_GL_CALL(glUniform1f(RendererObjectId, Other));
	}
	void operator=(const Vector2 & Other) const
	{
		GS_GL_CALL(glUniform2f(RendererObjectId, Other.X, Other.Y));
	}
	void operator=(const Vector3 & Other) const
	{
		GS_GL_CALL(glUniform3f(RendererObjectId, Other.X, Other.Y, Other.Z));
	}
	void operator=(const Vector4 & Other) const
	{
		GS_GL_CALL(glUniform4f(RendererObjectId, Other.X, Other.Y, Other.Z, Other.W));
	}
	void operator=(int Other) const
	{
		GS_GL_CALL(glUniform1i(RendererObjectId, Other));
	}
	void operator=(bool Other) const
	{
		GS_GL_CALL(glUniform1i(RendererObjectId, Other));
	}
	void operator=(const Matrix4 & Other) const
	{
		GS_GL_CALL(glUniformMatrix4fv(RendererObjectId, 1, GL_FALSE, Other.GetData()));
	}
};

