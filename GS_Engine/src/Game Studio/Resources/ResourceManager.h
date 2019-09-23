#pragma once

#include "Core.h"

#include "Object.h"

#include "Containers/FString.h"
#include "Containers/Id.h"

template<class RT>
class GS_API ResourceManager : public Object
{
protected:
	FString ResourcePath;

	static FString GetBaseResourcePath() { return "resources/"; }
public:
	virtual RT* GetResource(const FString& _Name) = 0;
	virtual void ReleaseResource(RT* _Resource) = 0;
};
