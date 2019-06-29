#pragma once

#include "Core.h"

GS_CLASS Id
{
public:
	Id() = default;
	explicit Id(const char * Text);
	~Id() = default;

	INLINE uint32 GetID() { return HashedString; }
	INLINE uint32 GetID() const { return HashedString; }
private:
	uint32 HashedString;

	static uint32 HashString(const char * Text);
};

