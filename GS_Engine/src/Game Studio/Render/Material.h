#pragma once

#include "MaterialInfo.h"
#include "Containers/DArray.hpp"
#include "Containers/Id.h"
#include "RAPI/GraphicsPipeline.h"
#include "Containers/Array.hpp"

class FString;
class MaterialResource;

/**
 * \brief Every instance of this class represents an individual material instance.
 * Which means material parameters can be modified and they will only affect this particular instance, not the material as a whole.
 *
 * Each material can reference up to 8 textures, and hold up to 32 bytes of dynamic parameter data.
 */
class Material
{
	static const char* GetPositionAttributeName() { return "inPos"; }
	static const char* GetTextureCoordinateAttributeName() { return "inTextCoord"; }

	MaterialResource* materialMaterialResource = nullptr;

	Array<MaterialParameter, 8> parameters;
	Array<class Texture*, 8> textures;

	byte vars[32];
public:
	explicit Material(const FString& _Name);
	virtual ~Material();

	[[nodiscard]] const char* GetMaterialName() const;

	//Writes the vertex shader code and fragment shader code to the passed in variables.
	void GetRenderingCode(FVector<RAPI::ShaderInfo>& shaders_) const; //TEMPORAL: manual for now, should then be automated.

	//Returns true if there is uniform set info and writes said info to the passed in string.
	bool GetUniformSetCode(FString& _Code); //TEMPORAL: manual for now, should then be automated.
	//Returns true if there is uniform set info and sets the size to the passed in int.
	bool GetUniformSetSize(size_t& _Size); //TEMPORAL: manual for now, should then be automated.

	void SetParameter(const Id& parameter_name_, RAPI::ShaderDataTypes data_type_, void* data_);
	void SetTexture(const Id& textureName, class Texture* texturePointer);

	MaterialResource* GetMaterialResource() { return materialMaterialResource; }
	[[nodiscard]] const decltype(textures)& GetTextures() const { return textures; }

	//Returns an array consisting of all of the material's dynamic parameters which change on a per instance basis. Used for building and updating shader data.
	Array<MaterialParameter, 8> GetMaterialDynamicParameters() const { return parameters; };

	//Returns whether this material has transparency. Which means it's see through.
	//true = has transparency.
	//false = doesn't have transparency. Is opaque.
	[[nodiscard]] bool GetHasTransparency() const;
	//Returns whether this material needs meshes to be rendered when seem from the front and from the back.
	//true = seen from front and back. (No winding culling).
	//false = seen only from "front". (Engine default vertex winding order (Clockwise)).
	[[nodiscard]] bool GetIsTwoSided() const;
};
