#pragma once

#include "Core.h"

#include "MeshRenderProxy.h"

struct Vector3;

GS_CLASS PointLightRenderProxy : public MeshRenderProxy
{
public:
	explicit PointLightRenderProxy(WorldObject * Owner);
	~PointLightRenderProxy();

	virtual void Draw() const;
protected:
	static Vector3 * GenVertices(const uint8 HSegments, const uint8 VSegments);
	static uint32 * GenIndices(const uint8 HSegments, const uint8 VSegments);

	static Vector3 * MeshLoc;
	static uint32 * IndexLoc;
};

