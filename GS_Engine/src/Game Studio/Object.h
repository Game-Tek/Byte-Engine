#pragma once

#include "Core.h"

GS_CLASS Object
{

public:
	Object() {};
	virtual ~Object() {};

	virtual void OnUpdate() {};

	[[nodiscard]] virtual const char* GetName() const = 0;
};