#version 410 core

in vec2 tTextCoord;

out vec4 FragColor;

uniform sampler2D ourTexture;

void main()
{
   FragColor = texture(ourTexture, tTextCoord);
}