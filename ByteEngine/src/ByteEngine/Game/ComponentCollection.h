#pragma once

#include "ByteEngine/Object.h"

class ComponentCollection : public Object
{
public:
	ComponentCollection() = default;
	~ComponentCollection() = default;

	using ComponentReference = uint32;
};
