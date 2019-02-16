#include "WorldObject.h"

WorldObject::WorldObject()
{
}

WorldObject::WorldObject(const Transform3 & Transform) : WorldPrimitive(Transform)
{
}

WorldObject::~WorldObject()
{
	delete RenderProxy;
}
