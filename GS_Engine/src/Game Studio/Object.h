#pragma once

#include "Core.h"

GS_CLASS Object
{
public:
	Object();
	virtual ~Object();

	virtual void OnUpdate() {};

	unsigned int GetId() const { return UUID; }
protected:
	unsigned int UUID;
};