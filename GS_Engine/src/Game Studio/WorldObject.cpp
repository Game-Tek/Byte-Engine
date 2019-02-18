#include "WorldObject.h"

WorldObject::WorldObject(const Transform3 & Transform) : WorldPrimitive(Transform)
{
}

WorldObject::~WorldObject()
{
	delete RenderProxy;
}
