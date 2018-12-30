#pragma once

#include "Core.h"

#include "RendererObject.h"

GS_CLASS Program : public RendererObject
{
public:
	Program();
	~Program();

	void Bind() const override;
};

