#pragma once

#include "Core.h"

struct ResourceData
{
	uint32 references = 0;

	uint32 IncrementReferences() { return ++references; }
	uint32 DecrementReferences() { return --references; }
	[[nodiscard]] uint32 GetReferenceCount() const { return references; }
};