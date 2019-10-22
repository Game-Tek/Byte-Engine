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
	class MaterialData : public ResourceData
	{
		FString VertexShaderCode;
		FString FragmentShaderCode;

	public:
		~MaterialData()
		{
		}

		void** WriteTo(size_t _Index, size_t _Bytes) override
		{
			switch (_Index)
			{
			case 3: VertexShaderCode = new char[_Bytes];
					return reinterpret_cast<void**>(&VertexShaderCode);
			case 6: VertexShaderCode = new char[_Bytes];
					return reinterpret_cast<void**>(&VertexShaderCode);
			default: ;
			}

			return nullptr;
		}

		[[nodiscard]] FString& GetVertexShaderCode() { return VertexShaderCode; }
		[[nodiscard]] FString& GetFragmentShaderCode() { return FragmentShaderCode; }

		friend Archive& operator<<(Archive& _O, MaterialData& _MD);
		friend Archive& operator>>(Archive& _I, MaterialData& _MD);
	};

	MaterialResource() = default;

	~MaterialResource()
	{
		delete Data;
	}

	bool LoadResource(const FString& _Path) override;
	void LoadFallbackResource(const FString& _Path) override;

	[[nodiscard]] const char* GetName() const override { return "Material Resource"; }

	[[nodiscard]] const char* GetResourceTypeExtension() const override { return ".gsmat"; }
};