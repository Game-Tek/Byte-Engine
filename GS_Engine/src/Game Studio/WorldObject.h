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
	explicit WorldObject(const Transform3 & Transform);
	virtual ~WorldObject() = default;

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

	GameInstance * GetGameInstance();
};