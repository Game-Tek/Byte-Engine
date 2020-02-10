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
	
public:
	BindingsSetDescriptor(const std::initializer_list<BindingDescriptor>& initializerList) : bindings(initializerList)
	{
	}

	[[nodiscard]] uint8 GetBindingsCount() const { return bindings.getLength(); }
	
	const BindingDescriptor& operator[](const uint8 i) const { return bindings[i]; }
};
