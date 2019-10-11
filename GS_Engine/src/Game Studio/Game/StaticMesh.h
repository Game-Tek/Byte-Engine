#pragma once

#include "Object.h"

class FString;
struct Model;
class StaticMeshResource;
class Material;

class StaticMesh : public Object
{
	StaticMeshResource* staticMeshResource = nullptr;
	Material* staticMeshMaterial = nullptr;
public:
	explicit StaticMesh(const FString& _Name);
	~StaticMesh();

	[[nodiscard]] const char* GetName() const override { return "Static Mesh"; }

	[[nodiscard]] Material* GetMaterial() const { return staticMeshMaterial; }
	[[nodiscard]] Model GetModel() const;

	void SetMaterial(Material* _NewMaterial) { staticMeshMaterial = _NewMaterial; }
};
