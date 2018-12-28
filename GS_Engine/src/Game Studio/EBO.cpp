#include "EBO.h"



EBO::EBO()
{
	glGenBuffers(1, &(unsigned int&)Id);
}


EBO::~EBO()
{
	glDeleteBuffers(1, &(unsigned int&)Id);
}

void EBO::Bind(int Usage)
{
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, Id);
	//glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(indices), indices, Usage);
}
