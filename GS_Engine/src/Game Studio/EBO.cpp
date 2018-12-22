#include "EBO.h"



EBO::EBO()
{
}


EBO::~EBO()
{
}

void EBO::Bind(int Usage = GL_STATIC_DRAW)
{
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, Id);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(indices), indices, Usage);
}
