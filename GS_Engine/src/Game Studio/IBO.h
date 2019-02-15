#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS IBO : public RendererObject

{
public:
	//Generates a new buffer.
	IBO(const void * Data, uint32 Count);
	~IBO();

	//Makes this buffer the currently bound buffer.
	void Bind() const override;

	uint32 GetCount() const { return IndexCount; }
private:
	uint32 IndexCount = 0;
};

