#pragma once

#include "GAL/RenderCore.h"
#include <GTSL/Id.h>

//Specifies a single shader parameter. Used to build uniform sets and to specify shader information.
struct MaterialParameter
{
	GTSL::Id64 ParameterName;
	//Specifies the type of the variable being referred to so we can build uniform sets and copy information.
	GAL::ShaderDataTypes ParameterDataType;
	//Pointer to the variable holding the data to be copied to the GPU.
	void* Data = nullptr;
};

struct MaterialInfo
{
};
