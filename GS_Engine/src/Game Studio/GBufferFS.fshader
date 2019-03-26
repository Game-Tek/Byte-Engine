#version 410 core


layout(location = 0) out vec3 outPosition;
layout(location = 1) out vec3 outNormal;
layout(location = 2) out vec3 outAlbedo;

//Vertex data input.

in vec3 tViewFragPos;
in vec2 tTextCoord;
in vec3 tNormal;

//Texture input.
uniform sampler2D ourTexture;

void main()
{
   outPosition = tViewFragPos;
   outNormal = tNormal;
   outAlbedo = vec3(0.2, 0.5, 0.6);
}