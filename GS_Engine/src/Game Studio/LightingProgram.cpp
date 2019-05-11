#include "LightingProgram.h"

LightingProgram::LightingProgram() : Program("W:/Game Studio/GS_Engine/src/Game Studio/LightingVS.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/LightingFS.fshader"), PositionTextureSampler(this, "uPosition"), NormalTextureSampler(this, "uNormal"), AlbedoTextureSampler(this, "uAlbedo")
{
}

LightingProgram::~LightingProgram()
{
}
