#pragma once

#include "Byte Engine/Core.h"
#include "Byte Engine/Object.h"

#include <GTSL/Math/Transform3.h>
#include <GTSL/Math/Vector3.h>

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

	void SetTransform(const GTSL::Transform3& _NewTransform) { Transform = _NewTransform; }
	[[nodiscard]] GTSL::Transform3& GetTransform() { return Transform; }
	void SetPosition(const GTSL::Vector3& _Pos) { Transform.Position = _Pos; }
	[[nodiscard]] const GTSL::Vector3& GetPosition() const { return Transform.Position; }

	static World* GetWorld();
protected:
	GTSL::Transform3 Transform;

	WorldObjectID ID = 0;
};
