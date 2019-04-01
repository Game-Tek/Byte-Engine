#include "ScreenQuad.h"
										//Position		   //UV		   //Position		  //UV		  //Position		  //UV		  //Position		 //UV
float ScreenQuad::SquareVertexData[] = { 1.0f, 1.0f, 0.0f, 1.0f, 1.0f, 1.0f, -1.0f, 0.0f, 1.0f, 0.0f, -1.0f, -1.0f, 0.0f, 0.0f, 0.0f, -1.0f, 1.0f, 0.0f, 0.0f, 1.0f };
uint8 ScreenQuad::SquareIndexData[] = { 0, 1, 2, 2, 3, 0 };

ScreenQuad::ScreenQuad() : MeshRenderProxy(new VBO(SquareVertexData, sizeof(SquareVertexData)), new IBO(SquareIndexData, 6), new VAO(sizeof(float) * 5))
{
	VertexArray->Bind();
	VertexArray->CreateVertexAttribute(3, GL_FLOAT, false, sizeof(float) * 3);
	VertexArray->CreateVertexAttribute(2, GL_FLOAT, false, sizeof(float) * 2);
}

ScreenQuad::~ScreenQuad()
{
}