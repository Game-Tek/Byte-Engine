#version 410 core

layout(location = 0) in vec3 inPos;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec2 inTextCoord;
layout(location = 3) in vec3 inTangent;
layout(location = 4) in vec3 inBiTangent;

uniform mat4 uView;
uniform mat4 uProjection;

out vec2 tTextCoord;

void main()
{
   gl_Position = vec4(inPos, 1.0) * uView * uProjection;

   tTextCoord = inTextCoord;
}