#version 410 core


layout(location = 0) in vec3 inPos;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec2 inTextCoord;
layout(location = 3) in vec3 inTangent;
layout(location = 4) in vec3 inBiTangent;

uniform mat4 uProjection;
uniform mat4 uView;
uniform mat4 uModel;

out vec2 tTextCoord;
out vec3 tNormal;

out vec3 FragPos;

void SetPassthroughVariables()
{
	tTextCoord = inTextCoord;
	tNormal = inNormal;
}

void main()
{
   gl_Position = uProjection * uView * uModel * vec4(inPos, 1.0);

   FragPos = vec3(uModel * vec4(inPos, 1.0));
}