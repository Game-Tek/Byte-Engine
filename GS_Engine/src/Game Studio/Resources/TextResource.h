#pragma once

#include "Resource.h"

#include "Containers/FString.h"

class TextResource final : public Resource
{
public:
	class TextResourceData final : public ResourceData
	{
		FString Text;

	public:
		friend OutStream& operator<<(OutStream& _OS, TextResourceData& _TRD)
		{
			_OS << _TRD.Text;
			return _OS;
		}

		friend InStream& operator>>(InStream& _IS, TextResourceData& _TRD)
		{
			_IS >> _TRD.Text;
			return _IS;
		}

		void** WriteTo(size_t _Index, size_t _Bytes) override
		{
			return nullptr;
		}
	};

private:
	TextResourceData data;

public:
	[[nodiscard]] const char* GetName() const override { return "Text Resource"; }

	[[nodiscard]] const TextResourceData& GetTextData() const { return data; }
	
private:
	[[nodiscard]] const char* getResourceTypeExtension() const override { return "txt"; }
	
	bool loadResource(const LoadResourceData& LRD_) override;
	void loadFallbackResource(const FString& _FullPath) override;
};