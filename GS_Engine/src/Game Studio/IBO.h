#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS IBO : RendererObject

{
public:
	//Generates a new buffer.
	IBO(const void * Data, unsigned int Count);
	~IBO();

	//Makes this buffer the currently bound buffer.
	void Bind() const;

	unsigned int GetCount() const { return IndexCount; }
	unsigned int * GetIndexArrayPointer() const { return IndexArray; }
private:
	unsigned int IndexCount;
	unsigned int * IndexArray;
};

