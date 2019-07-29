#pragma once

#include "Core.h"

#include "RAPI/Shader.h"
#include "VulkanBase.h"

#include "Containers/Tuple.h"

#include <vector>
#include "Containers/FString.h"

GS_CLASS VulkanShader final : public Shader
{
	static Tuple<std::vector<char>, size_t> GetShaderCode(const FString& _Name);

	Vk_Shader ShaderModule;
public:
	VulkanShader(VkDevice _Device, const FString& _Name, ShaderType _ShaderType);

	INLINE const Vk_Shader& GetVk_Shader() const { return ShaderModule; }
};