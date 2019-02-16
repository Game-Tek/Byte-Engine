#pragma once

#include "Core.h"

#include "Vertex.h"

//Bind then buffer data.

//			TERMINOLOGY
//	Count: how many something are there.
//	Size: size of the object in bytes.

GS_CLASS RendererObject
{
public:
	RendererObject();
	virtual ~RendererObject();

	virtual void Bind() const {} ;

	uint32 GetId() const { return RendererObjectId; }

protected:
	uint32 RendererObjectId = 0;
};