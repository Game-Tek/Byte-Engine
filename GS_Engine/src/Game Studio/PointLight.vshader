#version 410 core  

layout(location = 0) in vec3 inPos;

uniform mat4 uProjection;
uniform mat4 uView;
uniform mat4 uModel;

void main()
{
	gl_Position = uProjection * uView * uModel * vec4(inPos, 1.0);
}