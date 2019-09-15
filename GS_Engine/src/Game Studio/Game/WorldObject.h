#pragma once

#include "Core.h"

#include "Object.h"

#include "Math\Transform3.h"

class GameInstance;
class RenderProxy;

GS_CLASS WorldObject : public Object
{
public:
	WorldObject() = default;
	explicit WorldObject(const Transform3 & _Transform) : Transform(_Transform)
	{
	}

	virtual ~WorldObject() = default;

	[[nodiscard]] Transform3& GetTransform() { return Transform; }
protected:
	Transform3 Transform;
};