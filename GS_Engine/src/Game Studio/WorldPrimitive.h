#pragma once

#include "Core.h"

#include "Transform3.h"

GS_CLASS WorldPrimitive
{
public:
	WorldPrimitive();
	~WorldPrimitive();

	Transform3 GetTransform() const { return Transform; }
	Vector3 GetPosition() const { return Transform.Position; }
	Rotator GetRotation() const { return Transform.Rotation; }
	Vector3 GetScale() const { return Transform.Scale; }

	void SetTransform(const Transform3 & NewTransform) { Transform = NewTransform; }
	void SetPosition(const Vector3 & NewPosition) { Transform.Position = NewPosition; }
	void SetRotation(const Rotator & NewRotation) { Transform.Rotation = NewRotation; }
	void SetScale(const Vector3 & NewScale) { Transform.Scale = NewScale; }

	void AddDeltaPosition(const Vector3 & Delta) { Transform.Position += Delta; }

protected:
	Transform3 Transform;
};

