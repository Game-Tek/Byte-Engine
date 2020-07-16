#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/Id.h>
#include <GTSL/StaticString.hpp>

/**
 * \brief Used to specify a type of resource loader.
 *
 * This class will be instanced sometime during the application's lifetime to allow loading of some type of resource made possible by extension of this class.
 *
 * Every extension will allow for loading of 1 type of resource.
 */
class ResourceManager : public Object
{
public:
	ResourceManager() = default;

	struct ResourceLoadInfo
	{
		[[deprecated]] GTSL::StaticString<128> NameString;
		GTSL::Id64 Name;
	};
};