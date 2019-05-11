#include "PointLightProgram.h"

PointLightProgram::PointLightProgram() : Program("W:/Game Studio/GS_Engine/src/Game Studio/PointLight.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/PointLight.fshader"), AlbedoTextureSampler(this, "uAlbedo")
{
//	AlbedoTextureSampler.Set(0);
}

PointLightProgram::~PointLightProgram()
{
}
