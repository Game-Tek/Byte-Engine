#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Containers/FString.h"

GS_STRUCT ShaderCreateInfo
{
	ShaderType Type;
	FString ShaderName;
};

GS_CLASS Shader
{
	ShaderType Type;

public:
	Shader(ShaderType _Type) : Type(_Type)
	{
	}

	INLINE ShaderType GetShaderType() const { return Type; }

	virtual ~Shader() = default;
};