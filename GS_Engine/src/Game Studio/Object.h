#pragma once

#include "Core.h"

GS_CLASS Object
{
public:
	Object();
	virtual ~Object();

	virtual void OnUpdate() {};
};