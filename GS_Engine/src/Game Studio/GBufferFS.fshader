#version 410 core

//GBuffer out variables.

layout(location = 0) out vec3 outPosition;
layout(location = 1) out vec3 outNormal;
layout(location = 2) out vec3 outAlbedo;

//VertexShader input.

in vec3 tViewFragPos;
in vec2 tTextCoord;
in vec3 tNormal;

void main()
{
   outPosition = tViewFragPos;
   outNormal = tNormal;
   outAlbedo = vec3(0.8, 0.5, 0.2);
}