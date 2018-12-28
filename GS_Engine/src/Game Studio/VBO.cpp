#include "VBO.h"

VBO::VBO()
{
	glGenBuffers(1, & (unsigned int&)Id);
}


VBO::~VBO()
{
	glDeleteBuffers(1, &(unsigned int&)Id);
}

void VBO::Bind(int Usage)
{
	glBindBuffer(GL_ARRAY_BUFFER, Id);
	//glBufferData(GL_ARRAY_BUFFER, sizeof(vertices), vertices, Usage);
}