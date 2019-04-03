#pragma once

#include "Core.h"

#include "RendererObject.h"

class Texture;
struct ImageSize;

GS_CLASS FrameBuffer : public RendererObject
{
public:
	explicit FrameBuffer(const uint8 NumberOfColorAttachments);
	~FrameBuffer();

	//Sets this frame buffer as the currently bound frame buffer.
	void Bind() const override;
	//Sets this frame buffer as the currently bound read frame buffer.
	void BindForRead() const;
	//Sets this frame buffer as the currently bound write frame buffer.
	void BindForWrite() const;
	//Unbinds this frame buffer.
	void UnBind() const override;

	//Clears the currently bound frame buffer.
	static void Clear();
	//Copies content from one frame buffer to the other.
	static void CopyFrameBuffer(const ImageSize & Size);

	//Sets all of this frame buffer's color attachments as the bound draw targets.
	void SetAsDrawBuffer() const;

	//Unbinds the currently bound read frame buffer.
	static void UnBindRead();
	//Unbinds the currently bound write frame buffer.
	static void UnBindWrite();

	//Attaches a texture to one of this frame buffer's color attachments.
	void AttachTexture(const Texture & Texture);
	//Attaches a texture to one of this frame buffer's color attachments.
	void AttachTexture(Texture * Texture);

	//Sets the bound frame buffer's Index color attachment as the currently bound read texture/target.
	static void SetReadBuffer(const uint8 Index);

	//Returns the number of textures this frame buffer has.
	uint8 GetNumberOfBoundTextures() const { return BoundTextures; }

	//Returns a pointer to the array holding the active color attachments.
	uint32 * GetActiveColorAttachments() const { return ColorAttachments; }

private:
	//Keeps track of how many textures have been bound.
	uint8 BoundTextures = 0;

	//Points to the array holding the active color attachments.
	uint32 * ColorAttachments;

	//Returns a pointer to a dynamically allocated array holding the active color attachments.
	static uint32 * GenColorAttachments(const uint8 N);
};