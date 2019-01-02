#include "Texture.h"

#include "glad.h"

#include "Logger.h"

Texture::Texture(const char * ImageFilePath)
{
	glGenTextures(1, & RendererObjectId);								//Generate a buffer to store the texture.

	glBindTexture(GL_TEXTURE_2D, RendererObjectId);						//Bind the texture so all following texture setup calls have effect on this texture.

	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);		//Set texture wrapping method for the the S axis as GL_REPEAT.
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);		//Set texture wrapping method for the the T axis as GL_REPEAT.
	
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);	//Set texture minification filter as GL_LINEAR blend.
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);	//Set texture magnification filter as GL_LINEAR blend.

	unsigned int NumberOfChannels;
	
	unsigned char * ImageData = stbi_load(FileSystem::getPath("resources/textures/container.jpg").c_str(), & ImageDimensions.Width, & ImageDimensions.Height, & NumberOfChannels, 0);	//Import the image.

	if (ImageData)	//Check if image import succeeded. If so.
	{
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, TextureDimensions.Width, TextureDimensions.Height, 0, GL_RGB, GL_UNSIGNED_BYTE, ImageData);
		glGenerateMipmap(GL_TEXTURE_2D);
	}
	else
	{
		GS_LOG_WARNING("Failed to import texture!")
	}
	stbi_image_free(data);
}


Texture::~Texture()
{
	glDeleteTextures(1, & RendererObjectId);
}

void Texture::Bind() const
{
	glBindTexture(GL_TEXTURE_2D, RendererObjectId);
}

void Texture::ActivateTexture(unsigned short Index) const
{
	glActiveTexture(GL_TEXTURE0 + Index);
}