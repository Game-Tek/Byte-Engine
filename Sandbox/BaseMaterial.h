#pragma once

#include <Game Studio/Render/Material.h>

class BaseMaterial : public Material
{
public:
	BaseMaterial(const FString& _Name) : Material(_Name)
	{
	}

	[[nodiscard]] bool GetIsTwoSided() const override { return false; }
	[[nodiscard]] bool GetHasTransparency() const override { return false; }
	DArray<MaterialParameter> GetMaterialDynamicParameters() override { return DArray<MaterialParameter>(); }
};