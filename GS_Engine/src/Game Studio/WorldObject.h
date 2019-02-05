#pragma once

#include "Core.h"

#include "Object.h"

#include "Transform3.h"

GS_CLASS WorldObject : public Object
{
public:
	WorldObject();
	WorldObject(const Transform3 & Transform);

	Transform3 GetTransform() const { return Transform; }
	Vector3 GetPosition() const { return Transform.Position; }
	Rotator GetRotation() const { return Transform.Rotation; }
	Vector3 GetScale() const { return Transform.Scale; }

	void SetTransform(const Transform3 & NewTransform) { Transform = NewTransform; }
	void SetPosition(const Vector3 & NewPosition) { Transform.Position = NewPosition; }
	void SetRotation(const Rotator & NewRotation) { Transform.Rotation = NewRotation; }
	void SetScale(const Vector3 & NewScale) { Transform.Scale = NewScale; }

protected:
	Transform3 Transform;
};