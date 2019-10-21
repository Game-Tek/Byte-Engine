#pragma once

#include "Resource.h"

#include "Containers/FString.h"

class TextResource : public Resource
{
public:
	class TextResourceData : public ResourceData
	{
		FString Text;

	public:
		friend Archive& operator<<(Archive& _OS, TextResourceData& _TRD)
		{
			_OS << _TRD.Text;
			return _OS;
		}

		friend Archive& operator>>(Archive& _IS, TextResourceData& _TRD)
		{
			_IS >> _TRD.Text;
			return _IS;
		}

		void** WriteTo(size_t _Index, size_t _Bytes) override
		{
			return nullptr;
		}
	};

	const char* GetResourceTypeExtension() const override { return ".txt"; }
	const char* GetName() const override { return "Text Resource"; }
	bool LoadResource(const FString& _FullPath) override;
	void LoadFallbackResource(const FString& _FullPath) override;
};