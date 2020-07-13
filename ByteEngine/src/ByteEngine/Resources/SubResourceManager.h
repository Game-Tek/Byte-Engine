#pragma once

#include "ByteEngine/Object.h"
#include <GTSL/StaticString.hpp>

/**
 * \brief Used to specify a type of resource loader. When inherited it's functions implementation should load resources as per request
 * from the ResourceManager.
 *
 * This class will be instanced sometime during the application's lifetime to allow loading of some type of resource made possible by extension of this class.
 * 
 * Every extension will allow for loading of 1 type of resource specified with a pretty name by the GetResourceType() function. Users will request loading of
 * some type of resource by asking for a resource of this "pretty" name type.
 */
class SubResourceManager : public Object
{
public:
	explicit SubResourceManager(const char* resourceType)
	{	
	}
	
	~SubResourceManager() = default;

	struct ResourceLoadInfo
	{
		GTSL::StaticString<128> Name;
	};
	
protected:
};