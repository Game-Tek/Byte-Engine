#include "Program.h"

#include "glad.h"

#include "VertexShader.h"
#include "FragmentShader.h"


Program::Program()
{
	VertexShader VS;
	FragmentShader FS;

	RendererObjectId = glCreateProgram();
	glAttachShader(RendererObjectId, VS.GetId());
	glAttachShader(RendererObjectId, FS.GetId());
	glLinkProgram(RendererObjectId);
}


Program::~Program()
{
	glDeleteProgram(RendererObjectId);
}

void Program::Bind() const
{
	glUseProgram(RendererObjectId);
}