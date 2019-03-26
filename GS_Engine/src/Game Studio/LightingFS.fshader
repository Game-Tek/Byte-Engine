#version 330 core

in vec2 inTexCoords;

out vec4 FragColor;

uniform sampler2D uPosition;
uniform sampler2D uNormal;
uniform sampler2D uAlbedo;

void main()
{             
    FragColor = vec4(lighting, 1.0);
}