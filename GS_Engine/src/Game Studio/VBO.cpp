#include "VBO.h"

VBO::VBO()
{
	glGenBuffers(1, & Id);
}


VBO::~VBO()
{
}

void VBO::Bind(int Usage = GL_STATIC_DRAW)
{
	glBindBuffer(GL_ARRAY_BUFFER, Id);
	glBufferData(GL_ARRAY_BUFFER, sizeof(vertices), vertices, Usage);
}
