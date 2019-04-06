#pragma once

#include "Core.h"

#include "MeshRenderProxy.h"

GS_CLASS ScreenQuad : public MeshRenderProxy
{
public:
	ScreenQuad();
	~ScreenQuad();

	void Draw() override;

private:
	static float SquareVertexData[];
	static uint8 SquareIndexData[];
};

