#pragma once

#include "Core.h"

struct SerializeInfo;

class GS_API Object
{

public:
	Object() = default;
	virtual ~Object() = default;

	virtual void OnUpdate() {}

	[[nodiscard]] virtual const char* GetName() const = 0;
	virtual void Serialize(OutStream& _SI) const;
};