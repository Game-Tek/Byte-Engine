#pragma once

#include "Core.h"

#include "Object.h"

#include "DataTypes.h"

GS_CLASS WorldObject : public Object
{
public:
	Transform3 GetTransform() const { return Transform; }
	Vector3 GetPosition() const { return Transform.Location; }
	Rotator GetRotation() const { return Transform.Rotation; }
	Vector3 GetScale() const { return Transform.Size; }

	void SetTransform(const Transform3 & NewTransform) { Transform = NewTransform; }
	void SetPosition(const Vector3 & NewPosition) { Transform.Location = NewPosition; }
	void SetRotation(const Rotator & NewRotation) { Transform.Rotation = NewRotation; }
	void SetScale(const Vector3 & NewScale) { Transform.Size = NewScale; }

protected:
	Transform3 Transform;
};