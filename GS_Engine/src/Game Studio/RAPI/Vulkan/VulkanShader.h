#pragma once

#include "Core.h"

#include "RAPI/Shader.h"
#include "VulkanBase.h"

#include "Containers/Tuple.h"

#include <vector>
#include "Containers/FString.h"

MAKE_VK_HANDLE(VkShaderModule)

GS_CLASS Vk_Shader final : public VulkanObject
{
	VkShaderModule Shader = nullptr;

public:
	Vk_Shader(VkDevice _Device, Tuple<std::vector<char>, size_t> _Data);
	~Vk_Shader();

	INLINE VkShaderModule GetVkShaderModule() const { return Shader; }
};

GS_CLASS VulkanShader final : public Shader
{
	static Tuple<std::vector<char>, size_t> GetShaderCode(const FString& _Name);

	Vk_Shader ShaderModule;
public:
	VulkanShader(VkDevice _Device, const FString& _Name, ShaderType _ShaderType);

	INLINE const Vk_Shader& GetVk_Shader() const { return ShaderModule; }
};