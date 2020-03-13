#pragma once

#include "Core.h"

#include "Object.h"

#include "Math\Transform3.h"

class World;
class GameInstance;
class RenderProxy;

using WorldObjectID = uint64;

class RenderInfo;

class WorldObject : public Object
{
public:
	WorldObject() = default;
	virtual ~WorldObject() = default;

	virtual void Destroy(class World* ownerWorld) = 0;
	
	void SetID(WorldObjectID _ID) { ID = _ID; }

	[[nodiscard]] WorldObjectID GetID() const { return ID; }

	void SetTransform(const Transform3& _NewTransform) { Transform = _NewTransform; }
	[[nodiscard]] Transform3& GetTransform() { return Transform; }
	void SetPosition(const Vector3& _Pos) { Transform.Position = _Pos; }
	[[nodiscard]] const Vector3& GetPosition() const { return Transform.Position; }

	static World* GetWorld();
protected:
	Transform3 Transform;

	WorldObjectID ID = 0;
};
