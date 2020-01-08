#pragma once

#include "RAPI/RenderCore.h"
#include "Containers/Id.h"

//Specifies a single shader parameter. Used to build uniform sets and to specify shader information.
struct MaterialParameter
{
	Id ParameterName;
	//Specifies the type of the variable being referred to so we can build uniform sets and copy information.
	ShaderDataTypes ParameterDataType;
	//Pointer to the variable holding the data to be copied to the GPU.
	void* Data = nullptr;
};

struct MaterialInfo
{
};