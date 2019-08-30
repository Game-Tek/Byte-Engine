#pragma once

#include "Core.h"

#include "Math\Transform3.h"

GS_CLASS WorldPrimitive
{
public:
	WorldPrimitive() = default;
	explicit WorldPrimitive(const Transform3 & Transform);
	virtual ~WorldPrimitive() = default;

	Transform3 GetTransform() const { return Transform; }
	Vector3 GetPosition() const { return Transform.Position; }
	Vector3 GetScale() const { return Transform.Scale; }

	void SetTransform(const Transform3 & NewTransform) { Transform = NewTransform; }
	void SetPosition(const Vector3 & NewPosition) { Transform.Position = NewPosition; }
	void SetScale(const Vector3 & NewScale) { Transform.Scale = NewScale; }

	void AddDeltaPosition(const Vector3 & Delta) { Transform.Position += Delta; }

protected:
	Transform3 Transform;
};

