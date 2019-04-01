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

	void Bind() const override;
	void BindForRead() const;
	void BindForWrite() const;
	void UnBind() const override;

	//Clears the currently bound frame buffer.
	static void Clear();
	static void CopyFrameBuffer(const ImageSize & Size);

	void SetAsDrawBuffer() const;

	static void UnBindRead();
	static void UnBindWrite();

	void AttachTexture(const Texture & Texture);
	void AttachTexture(Texture * Texture);

	static void SetReadBuffer(const uint8 Index);

	uint8 GetNumberOfBoundTextures() const { return BoundTextures; }

	uint32 * GetActiveColorAttachments() const { return ColorAttachments; }

private:
	//Keeps track of how many textures have been bound.
	uint8 BoundTextures = 0;

	uint32 * ColorAttachments;

	static uint32 * GenColorAttachments(const uint8 N);
};