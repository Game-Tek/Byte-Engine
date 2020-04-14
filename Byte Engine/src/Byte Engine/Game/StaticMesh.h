#pragma once

#include "Object.h"
#include "Resources/ResourceReference.h"

class GTSL::String;
struct Model;

class StaticMesh final : public Object
{
	ResourceReference staticMeshResource;
public:
	explicit StaticMesh(const GTSL::String& _Name);
	~StaticMesh();

	[[nodiscard]] const char* GetName() const override { return "Static Mesh"; }

	[[nodiscard]] Model GetModel() const;
};
