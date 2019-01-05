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
	void Bind() const;

	unsigned int GetId() const { return RendererObjectId; }

protected:
	unsigned int RendererObjectId = 0;
};