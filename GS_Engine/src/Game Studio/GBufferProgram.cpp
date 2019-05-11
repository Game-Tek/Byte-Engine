#include "GBufferProgram.h"

GBufferProgram::GBufferProgram() : Program("W:/Game Studio/GS_Engine/src/Game Studio/GBufferVS.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/GBufferFS.fshader"), 
									ModelMatrix(this, "uModel"), 
									ViewMatrix(this, "uView"), 
									ProjMatrix(this, "uProjection")
{
}

GBufferProgram::~GBufferProgram()
{
}
