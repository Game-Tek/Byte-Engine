#pragma once

#include "Core.h"

#include "RAPI/RenderCore.h"

#include <initializer_list>
#include "Containers/DArray.hpp"

struct BindingDescriptor
{
	/**
	 * \brief Specifies the array size if this binding is of array type. Else it is 0.
	 */
	uint32 Count = 0;
	
	/**
	 * \brief Defines the type of the binding.
	 */
	RAPI::UniformType Type;
};

class BindingsSetDescriptor
{
	DArray<BindingDescriptor> bindings;
	RAPI::ShaderType shaderType;
	
public:
	BindingsSetDescriptor(const std::initializer_list<BindingDescriptor>& initializerList, RAPI::ShaderType shader) : bindings(initializerList), shaderType(shader)
	{
	}

	[[nodiscard]] auto begin() const { return bindings.begin(); }
	[[nodiscard]] auto end() const { return bindings.end(); }
	
	[[nodiscard]] uint8 GetBindingsCount() const { return bindings.getLength(); }

	[[nodiscard]] RAPI::ShaderType GetShaderType() const { return shaderType; }
	
	const BindingDescriptor& operator[](const uint8 i) const { return bindings[i]; }
};
