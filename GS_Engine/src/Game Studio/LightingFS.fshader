#version 410 core

in vec2 tTexCoords;

out vec4 FragColor;

//GBuffer input uniform variables.

uniform sampler2D uPosition;
uniform sampler2D uNormal;
uniform sampler2D uAlbedo;

void main()
{             
    FragColor = texture(uPosition, tTexCoords);
	//FragColor = vec4(0.65, 0.1, 0.3, 1);
}