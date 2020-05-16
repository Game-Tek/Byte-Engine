#pragma once

#include "Core.h"

#include "GAL/RenderCore.h"

#include <initializer_list>
#include <GTSL/FixedVector.hpp>

struct BindingDescriptor
{
	/**
	 * \brief Specifies the array size if this binding is of array type. Else it is 0.
	 */
	uint32 Count = 0;
	
	/**
	 * \brief Defines the type of the binding.
	 */
	GAL::BindingType Type;
};

class BindingsSetDescriptor
{
	GTSL::FixedVector<BindingDescriptor> bindings;
	GAL::ShaderType shaderType;
	
public:
	BindingsSetDescriptor(const std::initializer_list<BindingDescriptor>& initializerList, GAL::ShaderType shader) : bindings(initializerList), shaderType(shader)
	{
	}

	[[nodiscard]] auto begin() const { return bindings.begin(); }
	[[nodiscard]] auto end() const { return bindings.end(); }
	
	[[nodiscard]] uint8 GetBindingsCount() const { return bindings.GetLength(); }

	[[nodiscard]] GAL::ShaderType GetShaderType() const { return shaderType; }
	
	const BindingDescriptor& operator[](const uint8 i) const { return bindings[i]; }
};
