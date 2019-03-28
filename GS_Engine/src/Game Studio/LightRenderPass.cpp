#include "LightRenderPass.h"

LightRenderPass::LightRenderPass(Renderer * RendererOwner) : RenderPass(RendererOwner), LightingPassProgram("W:\Game Studio\GS_Engine\src\Game Studio\LightingVS.vshader", "W:\Game Studio\GS_Engine\src\Game Studio\LightingFS.fshader")
{
}

LightRenderPass::~LightRenderPass()
{
}