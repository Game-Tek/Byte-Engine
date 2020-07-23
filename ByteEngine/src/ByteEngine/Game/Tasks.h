#pragma once

#include "ByteEngine/Core.h"
#include <GTSL/Id.h>

enum class AccessType : uint8 { READ = 1, READ_WRITE = 4 };

struct TaskInfo
{
};

struct TaskDescriptor
{
	GTSL::Id64 System;
	AccessType Access;
};
