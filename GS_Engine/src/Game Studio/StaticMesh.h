#pragma once

#include "Core.h"

#include "Object.h"

#include "DArray.hpp"

GS_CLASS StaticMesh : public Object
{
	DArray<unsigned int>(5) Meshes;
};