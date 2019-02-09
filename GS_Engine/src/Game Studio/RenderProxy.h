#pragma once

#include "Core.h"

#include "VBO.h"
#include "IBO.h"

#include "WorldObject.h"

GS_CLASS RenderProxy
{
public:
	RenderProxy();
	RenderProxy(WorldObject * Owner) : Owner(Owner)
	{
	}

protected:
	WorldObject * Owner = nullptr;

	VBO * VertexBuffer = nullptr;
	IBO * IndexBuffer = nullptr;
};