#pragma once

#include "Core.h"

#include "Vertex.h"

#include "DataTypes.h"

//Bind then buffer data.


GS_CLASS Buffer
{
public:
	virtual void Bind();
	virtual void Enable();

protected:
	unsigned short Id;
};