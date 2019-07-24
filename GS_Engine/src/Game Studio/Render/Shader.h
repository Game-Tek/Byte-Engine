#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "FString.h"

GS_STRUCT ShaderCreateInfo
{
	ShaderType Type;
	String ShaderName;
};

GS_CLASS Shader
{
	ShaderType Type;
public:
	Shader(ShaderType _Type) : Type(_Type)
	{
	}

	INLINE ShaderType GetShaderType() const { return Type; }

	virtual ~Shader() {};
};