#pragma once

#include "Core.h"

#include "RendererObject.h"

#include <string>

#define GL_FRAGMENT_SHADER 0x8B30
#define GL_VERTEX_SHADER 0x8B31

GS_CLASS Shader : public RendererObject
{
public:
	Shader(unsigned int ShaderType, const char * ShaderPath);
	~Shader();

	std::string ReadShader(const char * Path);
};

