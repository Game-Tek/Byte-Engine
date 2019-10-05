#pragma once

#include "MaterialInfo.h"
#include "Containers/DArray.hpp"

class FString;

class Material
{
	static const char* GetPositionAttributeName() { return "inPos"; }
	static const char* GetTextureCoordinateAttributeName() { return "inTextCoord"; }

public:
	virtual ~Material() = default;

	virtual const char* GetMaterialName() = 0;

	//Writes the vertex shader code and fragment shader code to the passed in variables.
	virtual void GetRenderingCode(FString& _VertexCode, FString& _FragmentCode) = 0;  //TEMPORAL: manual for now, should then be automated.
	//Returns true if there is uniform set info and writes said info to the passed in string.
	virtual bool GetUniformSetCode(FString& _Code) = 0; //TEMPORAL: manual for now, should then be automated.
	//Returns true if there is uniform set info and sets the size to the passed in int.
	virtual bool GetUniformSetSize(size_t& _Size) = 0; //TEMPORAL: manual for now, should then be automated.

	virtual DArray<MaterialParameter> GetMaterialParameters() = 0;

	[[nodiscard]] virtual bool GetHasTransparency() const = 0;
	[[nodiscard]] virtual bool GetIsTwoSided() const = 0;
};