#pragma once

#include "Delegate.h"

class nApplication
{
	
public:
	struct ApplicationCreateInfo
	{
	};
	explicit nApplication(const ApplicationCreateInfo& applicationCreateInfo);

	virtual void Update() = 0;

	virtual void Close() = 0;
};
