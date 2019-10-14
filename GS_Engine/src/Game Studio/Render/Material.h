#pragma once

#include "MaterialInfo.h"
#include "Containers/DArray.hpp"

class FString;
class MaterialResource;

class Material
{
	static const char* GetPositionAttributeName() { return "inPos"; }
	static const char* GetTextureCoordinateAttributeName() { return "inTextCoord"; }

	MaterialResource* materialMaterialResource = nullptr;
public:
	explicit Material(const FString& _Name);
	virtual ~Material() = default;

	[[nodiscard]] virtual const char* GetMaterialName() const;

	//Writes the vertex shader code and fragment shader code to the passed in variables.
	void GetRenderingCode(char** _VS, char** _FS) const;  //TEMPORAL: manual for now, should then be automated.

	//Returns true if there is uniform set info and writes said info to the passed in string.
	bool GetUniformSetCode(FString& _Code); //TEMPORAL: manual for now, should then be automated.
	//Returns true if there is uniform set info and sets the size to the passed in int.
	bool GetUniformSetSize(size_t& _Size); //TEMPORAL: manual for now, should then be automated.

	//Returns an array consisting of all of the material's dynamic parameters which change on a per instance basis. Used for building and updating shader data.
	virtual DArray<MaterialParameter> GetMaterialDynamicParameters() = 0;

	//Returns whether this material has transparency. Which means it's see through.
	//true = has transparency.
	//false = doesn't have transparency. Is opaque.
	[[nodiscard]] virtual bool GetHasTransparency() const = 0;
	//Returns whether this material needs meshes to be rendered when seem from the front and from the back.
	//true = seen from front and back. (No winding culling).
	//false = seen only from "front". (Engine default vertex winding order (Clockwise)).
	[[nodiscard]] virtual bool GetIsTwoSided() const = 0;
};