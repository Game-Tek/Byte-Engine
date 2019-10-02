#pragma once

#include "Containers/Array.hpp"

class FString;

class Material
{
	static const char* GetPositionAttributeName() { return "inPos"; }
	static const char* GetTextureCoordinateAttributeName() { return "inTextCoord"; }

protected:
	bool HasTransparency = false;
	bool IsTwoSided = false;
	bool CastsShadows = true;

public:
	virtual ~Material() = default;

	virtual const char* GetMaterialName() = 0;

	//Writes the vertex shader code and fragment shader code to the passed in variables.
	virtual void GetRenderingCode(FString& _VertexCode, FString& _FragmentCode) = 0;
	//Returns true if there is uniform set info and writes said info to the passed in string.
	virtual bool GetUniformSetCode(FString& _Code) = 0;
	//Returns true if there is uniform set info and sets the size to the passed in int.
	virtual bool GetUniformSetSize(size_t& _Size) = 0;

	virtual void CreateResources() = 0;

	[[nodiscard]] bool GetHasTransparency() const { return HasTransparency; }
	[[nodiscard]] bool GetIsTwoSided() const { return IsTwoSided; }
};