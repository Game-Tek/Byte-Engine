#pragma once

#include "Core.h"

GS_CLASS Id
{
public:
	Id() = default;
	explicit Id(const char * Text);
	~Id() = default;

private:
	uint32 HashedString;

	static uint32 HashString(const char * Text);
};

