#include "FrameGraph.h"

FrameGraph::FrameGraph()
{
}

FrameGraph::~FrameGraph()
{
}

void FrameGraph::Execute()
{
	for (uint8 i = 0; i < RenderPasses.length(); i++)
	{
		RenderPasses[i].Execute();
	}
}

void FrameGraph::AddRenderPass(const RenderPass& _RP)
{
	RenderPasses.push_back(_RP);
}

void RenderPass::Execute()
{
	for (uint8 i = 0; i < SubPasses.length(); i++)
	{
		SubPasses[i].Execute();
	}
}

void RenderPass::AddSubPass(const SubPass& _SP)
{
	SubPasses.push_back(_SP);
}
