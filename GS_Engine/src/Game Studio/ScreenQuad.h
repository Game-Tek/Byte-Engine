#pragma once

#include "Core.h"

#include "MeshRenderProxy.h"

GS_CLASS ScreenQuad : public MeshRenderProxy
{
public:
	ScreenQuad();
	~ScreenQuad();

private:
	static float SquareVertexData[];
	static uint8 SquareIndexData[];
};

