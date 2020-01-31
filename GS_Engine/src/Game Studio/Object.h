#pragma once

#include "Core.h"

class Object
{
public:
	Object() = default;
	virtual ~Object() = default;

	virtual void OnUpdate()
	{
	}

	[[nodiscard]] virtual const char* GetName() const = 0;
};
