#pragma once

#include "Object.h"
#include "Resources/ResourceReference.h"

class FString;
struct Model;
class StaticMeshResource;
class Material;

class StaticMesh : public Object
{
	ResourceReference staticMeshResource;
public:
	explicit StaticMesh(const FString& _Name);
	~StaticMesh();

	[[nodiscard]] const char* GetName() const override { return "Static Mesh"; }

	[[nodiscard]] Model GetModel() const;
};
