#version 410 core

layout (location = 0) in vec3 inPos;
layout (location = 1) in vec3 inNormal;
layout (location = 2) in vec2 inTextCoord;

out vec2 tTextCoord;

void main()
{
   gl_Position = vec4(inPos.x, inPos.y, inPos.z, 1.0);

   tTextCoord = inTextCoord;
}