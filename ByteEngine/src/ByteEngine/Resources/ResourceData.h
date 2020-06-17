#pragma once

#include "ByteEngine/Core.h"

struct ResourceHandle
{
	uint32 IncrementReferences() { return ++references; }
	uint32 DecrementReferences() { return --references; }
	[[nodiscard]] uint32 GetReferenceCount() const { return references; }

protected:
	uint32 references = 0;
};