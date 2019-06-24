#pragma once

#include "FVector.hpp"
#include "String.h"

enum class PassType : uint8
{
	PASS_TYPE_GRAPHICS,
	PASS_TYPE_COMPUTE
};

class Resource
{
	String Name;
};

class Pass
{
	String Name;

	void * Execute;

public:
	void SetExecute();
};

class RenderPass : public Pass
{
	FVector<SubPass> SubPasses;

	FVector<Resource> InResources;
	FVector<Resource> OutResources;
public:
	RenderPass(const String & _Name, PassType _PT);
	~RenderPass();

	void Execute();

	void AddSubPass(const SubPass& _SP);

	void AddInResource();
	void AddOutResource();
};

class SubPass : public Pass
{
public:
	SubPass(const String& _Name);
	~SubPass();

	void Execute();
};

class FrameGraph
{
	FVector<RenderPass> RenderPasses;

public:
	FrameGraph();
	~FrameGraph();

	void Execute();

	void AddRenderPass(const RenderPass& _RP);
};

