#pragma once

#include "Resource.h"

/*
 * Vertex Shader Parameter Collection
 * Vertex Shader Code
 * Fragment Shader Parameter Collection
 * Fragment Shader Code
 */
class MaterialResource : public Resource
{
public:
	class MaterialData final : public ResourceData
	{
	public:
		FString VertexShaderCode;
		FString FragmentShaderCode;

		bool HasTransparency = false;
		bool IsTwoSided = false;

		~MaterialData() = default;

		[[nodiscard]] const FString& GetVertexShaderCode() const { return VertexShaderCode; }
		[[nodiscard]] const FString& GetFragmentShaderCode() const { return FragmentShaderCode; }

		friend OutStream& operator<<(OutStream& _O, MaterialData& _MD);
		friend InStream& operator>>(InStream& _I, MaterialData& _MD);
	};

private:
	MaterialData data;

public:

	MaterialResource() = default;

	~MaterialResource() = default;

	[[nodiscard]] const MaterialData& GetMaterialData() const { return data; }

	bool loadResource(const LoadResourceData& LRD_) override;
	//WILL BE REMOVED ONCE RESOURCE LOADING IS RESTRUCTURED.
	void makeFromData(ResourceData& resourceData) override;
	void loadFallbackResource(const FString& _Path) override;

	[[nodiscard]] const char* GetName() const override { return "Material Resource"; }

	[[nodiscard]] const char* getResourceTypeExtension() const override { return "gsmat"; }
};
