#version 410 core

layout(location = 0) out vec3 outPosition;

out vec4 FragColor;

uniform sampler2D uAlbedo;

void main()
{
	FragColor = vec4(1, 1, 1, 1);
}