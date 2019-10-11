#pragma once

#include "Resource.h"

class MaterialResource : public Resource
{
	class MaterialData : public ResourceData
	{
		char* VertexShaderCode = nullptr;
		char* FragmentShaderCode = nullptr;
		int32 ShaderDynamicParameters = 0;

	public:
		~MaterialData()
		{
			delete[] VertexShaderCode;
			delete[] FragmentShaderCode;
		}

		void** WriteTo(size_t _Index, size_t _Bytes) override
		{
			switch (_Index)
			{
			case 0: VertexShaderCode = new char[_Bytes];
					return reinterpret_cast<void**>(&VertexShaderCode);
			case 1: VertexShaderCode = new char[_Bytes];
					return reinterpret_cast<void**>(&VertexShaderCode);
			case 2:
					return reinterpret_cast<void**>(&ShaderDynamicParameters);
			default: ;
			}

			return nullptr;
		}
	};

public:
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