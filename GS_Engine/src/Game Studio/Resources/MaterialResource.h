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

		void* WriteTo(size_t _Index, size_t _Bytes) override
		{
			switch (_Index)
			{
			case 0: return new char[_Bytes];
			case 1: return new char[_Bytes];
			case 2: return &ShaderDynamicParameters;
			default: ;
			}

			return nullptr;
		}
	};

public:
	MaterialResource() = default;

	~MaterialResource()
	{
		delete SCAST(MaterialData*, Data);
	}

	bool LoadResource(const FString& _Path) override;
	void LoadFallbackResource(const FString& _Path) override;

	[[nodiscard]] size_t GetDataSize() const override;

	[[nodiscard]] const char* GetResourceTypeExtension() const override { return ".gsmat"; }
protected:
};