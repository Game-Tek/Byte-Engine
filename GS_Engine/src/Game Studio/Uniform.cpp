#include "Uniform.h"

#include "glad.h"

#include "GL.h"

Uniform::Uniform(const Program & Progr, const char * UniformName)
{
	GS_GL_CALL(RendererObjectId = glGetUniformLocation(Progr.GetId(), UniformName));
}


Uniform::~Uniform()
{
}
