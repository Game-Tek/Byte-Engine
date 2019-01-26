#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS IBO : public RendererObject

{
public:
	//Generates a new buffer.
	IBO(const void * Data, unsigned int Count);
	~IBO();

	//Makes this buffer the currently bound buffer.
	void Bind() const override;

	unsigned int GetCount() const { return IndexCount; }
private:
	unsigned int IndexCount = 0;
};

