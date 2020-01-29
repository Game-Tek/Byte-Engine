#pragma once

#include "Core.h"

class GS_API Object
{
public:
	Object() = default;
	virtual ~Object() = default;

	virtual void OnUpdate()
	{
	}

	[[nodiscard]] virtual const char* GetName() const = 0;
};
