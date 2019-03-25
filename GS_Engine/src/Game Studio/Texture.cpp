#include "Texture.h"

#include <GLAD/glad.h>

#include "Logger.h"

#include "GL.h"

#include <stb image/stb_image.h>

Texture::Texture(const char * ImageFilePath)
{
	GS_GL_CALL(glGenTextures(1, & RendererObjectId));								//Generate a buffer to store the texture.

	GS_GL_CALL(glBindTexture(GL_TEXTURE_2D, RendererObjectId));						//Bind the texture so all following texture setup calls have effect on this texture.

	GS_GL_CALL(glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT));		//Set texture wrapping method for the the S axis as GL_REPEAT.
	GS_GL_CALL(glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT));		//Set texture wrapping method for the the T axis as GL_REPEAT.
	
	GS_GL_CALL(glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST));	//Set texture minification filter as GL_LINEAR blend.
	GS_GL_CALL(glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR));	//Set texture magnification filter as GL_LINEAR blend.

	int NumberOfChannels;

	//stbi_set_flip_vertically_on_load(true);

	unsigned char * ImageData = stbi_load(ImageFilePath, & (int &)TextureDimensions.Width, & (int &)TextureDimensions.Height, & NumberOfChannels, 3);	//Import the image.

	if (ImageData)	//Check if image import succeeded. If so.
	{
		GS_GL_CALL(glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, TextureDimensions.Width, TextureDimensions.Height, 0, GL_RGB, GL_UNSIGNED_BYTE, ImageData));
		GS_GL_CALL(glGenerateMipmap(GL_TEXTURE_2D));
		GS_GL_CALL(glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_LINEAR));
	}
	else
	{
		GS_LOG_WARNING("Failed to import texture at, %s", ImageFilePath)
	}
	stbi_image_free(ImageData);
}

Texture::~Texture()
{
	GS_GL_CALL(glDeleteTextures(1, & RendererObjectId));
}

void Texture::Bind() const
{
	GS_GL_CALL(glBindTexture(GL_TEXTURE_2D, RendererObjectId));
}

void Texture::UnBind() const
{
	GS_GL_CALL(glBindTexture(GL_TEXTURE_2D, 0));
}

void Texture::SetActiveTextureUnit(uint8 Index)
{
	GS_GL_CALL(glActiveTexture(GL_TEXTURE0 + Index));
}