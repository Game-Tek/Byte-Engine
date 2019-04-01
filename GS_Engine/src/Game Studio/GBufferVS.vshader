#version 410 core

//Input Vertex Variables.

layout(location = 0) in vec3 inPos;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec2 inTextCoord;
layout(location = 3) in vec3 inTangent;
layout(location = 4) in vec3 inBiTangent;

uniform mat4 uProjection;
uniform mat4 uView;
uniform mat4 uModel;

//VertexShader out variables.

out vec3 tViewFragPos;
out vec2 tTextCoord;
out vec3 tNormal;

void SetPassthroughVariables()
{
	tViewFragPos = vec3(uView * vec4(inPos, 1.0));
	tTextCoord = inTextCoord;
	tNormal = inNormal;
}

void main()
{
   gl_Position = uProjection * uView * uModel * vec4(inPos, 1.0);

   tViewFragPos = vec3(uView * uModel * vec4(inPos, 1.0));
}