#pragma once

#include "Object.h"

namespace GTSL {
	class String;
}

struct Model;

class StaticMesh final : public Object
{
public:
	explicit StaticMesh(const GTSL::String& _Name);
	~StaticMesh();

	[[nodiscard]] const char* GetName() const override { return "Static Mesh"; }

	[[nodiscard]] Model GetModel() const;
};
