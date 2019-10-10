#pragma once

#include "Core.h"

#include "Object.h"

#include "Containers/FString.h"

//Base class representation of all types of resources that can be loaded into the engine.

class GS_API Resource : public Object
{
public:
	Resource() = default;

	virtual ~Resource() = default;

	//Returns the size of the data.
	[[nodiscard]] virtual size_t GetDataSize() const = 0;

	void IncrementReferences() { ++References; }
	void DecrementReferences() { --References; }
	[[nodiscard]] uint16 GetReferenceCount() const { return References; }

	virtual bool LoadResource(const FString& _FullPath) = 0;
	virtual void LoadFallbackResource(const FString& _FullPath) = 0;

	//Must return the extension name for the extension type, MUST contain the dot.
	//IE: ".gsasset". NOT "gsasset".
	[[nodiscard]] virtual const char* GetResourceTypeExtension() const = 0;

protected:
	void* Data = nullptr;

	uint16 References = 0;
};

struct ResourceData
{
	virtual ~ResourceData() = default;

	virtual void* WriteTo(size_t _Index, size_t _Bytes) = 0;
};

struct FileDescriptor
{
	FString DirectoryAndFileNameWithExtension;
	OutStream& OutStream;
};

struct FileElementDescriptor
{
	uint64 Bytes = 0;
	void* Data = nullptr;
};