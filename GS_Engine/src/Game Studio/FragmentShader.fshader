#version 410 core

//Vertex data input.
in vec2 tTextCoord;
in vec3 tNormal;

//Calulated position input.
in vec3 FragPos;

//Resulting color.
out vec4 FragColor;

//Texture input.
uniform sampler2D ourTexture;

void main()
{
   FragColor = vec4(0.18, 0.8, 0.44, 1);
}