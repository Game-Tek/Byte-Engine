#pragma once

#include "Byte Engine/Core.h"
#include "Byte Engine/Object.h"

#include <GTSL/Id.h>

class WorldObject : public Object
{
	GTSL::Id32 type;
	uint32 ID = 0;
};
