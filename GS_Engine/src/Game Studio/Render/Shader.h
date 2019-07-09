#pragma once

#include "Core.h"

#include "RenderCore.h"

GS_STRUCT ShaderCreateInfo
{
	ShaderType Type;
	String ShaderName;
};

GS_CLASS Shader
{
	ShaderType Type;

	Shader(const ShaderCreateInfo& _SI) : Type(_SI.Type)
	{
	}
public:

	INLINE ShaderType GetShaderType() const { return Type; }

	virtual ~Shader();
};