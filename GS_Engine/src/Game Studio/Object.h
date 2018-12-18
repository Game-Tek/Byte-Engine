#pragma once

#include "Core.h"

#include "String.h"

GS_CLASS Object
{
public:
	//Methods
	Object();
	~Object();

	void OnUpdate();

private:
	bool CanTick;
};