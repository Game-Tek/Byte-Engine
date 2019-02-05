#pragma once

#include "Core.h"

#include "VBO.h"
#include "IBO.h"

GS_CLASS RenderProxy
{
private:
	VBO * VertexBuffer = nullptr;
	IBO * IndexBuffer = nullptr;
};