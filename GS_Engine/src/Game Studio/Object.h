#pragma once

#include "Core.h"

class GS_API Object
{

public:
	Object() {};
	virtual ~Object() {};

	virtual void OnUpdate() {};

	[[nodiscard]] virtual const char* GetName() const = 0;
};