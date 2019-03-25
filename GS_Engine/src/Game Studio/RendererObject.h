#pragma once

#include "Core.h"

//Bind then buffer data.

//			TERMINOLOGY
//	Count: how many something are there.
//	Size: size of the object in bytes.

GS_CLASS RendererObject
{
public:
	RendererObject() = default;
	virtual ~RendererObject() = default;

	virtual void Bind() const {} ;
	virtual void UnBind() const {};

	uint32 GetId() const { return RendererObjectId; }

protected:
	uint32 RendererObjectId = 0;
};