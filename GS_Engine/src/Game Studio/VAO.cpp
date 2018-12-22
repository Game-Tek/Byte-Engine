#include "VAO.h"



VAO::VAO()
{
}


VAO::~VAO()
{
}

void VAO::Bind()
{
	glBindVertexArray(Id);
}

void VAO::CreateVertexAttribute()
{
	glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 3 * sizeof(float), (void*)0);
	glEnableVertexAttribArray(0);
}