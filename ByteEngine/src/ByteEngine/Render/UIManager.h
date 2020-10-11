#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Extent.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/KeepVector.h>
#include <GTSL/RGB.h>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vector2.h>


#include "RenderGroup.h"
#include "ByteEngine/Id.h"

enum class Alignment : uint8
{
	LEFT, CENTER, RIGHT
};

class Button : public Object
{
public:

	void SetMaterial(const ComponentReference newMat) { material = newMat; }
	
private:
	ComponentReference material;
};

class Square
{
public:
	Square() = default;

	void SetAspectRatio(const GTSL::Vector2 newAspectRatio) { aspectRatio = newAspectRatio; }
	[[nodiscard]] GTSL::Vector2 GetAspectRatio() const { return aspectRatio; }
	
	void SetColor(const GTSL::RGBA newColor) { color = newColor; }
	[[nodiscard]] GTSL::RGBA GetColor() const { return color; }
	void SetAlignment(const Alignment newAlignment) { alignment = newAlignment; }
	Alignment GetAlignment() const { return alignment; }

private:
	GTSL::RGBA color;
	GTSL::Vector2 aspectRatio;
	float32 rotation;
	Alignment alignment;
};

class TexturePrimitive
{
public:
	TexturePrimitive() = default;

	void SetSize(const GTSL::Vector2 newSize) { size = newSize; }
	[[nodiscard]] GTSL::Vector2 GetSize() const { return size; }

	void SetColor(const GTSL::RGBA newColor) { color = newColor; }
	[[nodiscard]] GTSL::RGBA GetColor() const { return color; }
	
	void SetTexture(const ComponentReference newTexture) { textureHandle = newTexture; }
	
private:
	GTSL::RGBA color;
	GTSL::Vector2 size;
	float32 rotation;

	ComponentReference textureHandle;
};

class TextPrimitive
{
public:
	TextPrimitive() = default;

	void SetSize(const GTSL::Vector2 newSize) { size = newSize; }
	[[nodiscard]] GTSL::Vector2 GetSize() const { return size; }

	void SetColor(const GTSL::RGBA newColor) { color = newColor; }
	[[nodiscard]] GTSL::RGBA GetColor() const { return color; }
	
	void SetString(const GTSL::Range<UTF8*> newText) { rawString = newText; }
	
private:
	GTSL::RGBA color;
	GTSL::Vector2 size;
	float32 rotation;

	GTSL::String<BE::PAR> rawString;
};

class Canvas : public Object
{
public:
	Canvas();

	void SetExtent(const GTSL::Extent2D newExtent) { realExtent = newExtent; }

	uint16 AddOrganizer(const Id name);

	uint16 AddSquare(const uint16 organizer)
	{
		return squaresPerOrganizer[organizer].Emplace();
	}

	void SetSquareAspectRatio(const uint16 organizer, const uint16 square, const GTSL::Vector2 aspectRatio)
	{
		squaresPerOrganizer[organizer][square].SetAspectRatio(aspectRatio);
	}

	void SetSquareColor(const uint16 organizer, const uint16 square, const GTSL::RGBA color)
	{
		squaresPerOrganizer[organizer][square].SetColor(color);
	}

	void SetSquareAlignment(const uint16 organizer, const uint16 square, const Alignment newAlignment)
	{
		squaresPerOrganizer[organizer][square].SetAlignment(newAlignment);
	}

	uint16 AddButton(const ComponentReference organizer, const Id name, const Alignment alignment = Alignment::CENTER);
	void SetOrganizerAspectRatio(const uint16 organizer, GTSL::Vector2 aspectRatio)
	{
		organizerAspectRatios[organizer] = aspectRatio;
	}

	GTSL::Extent2D GetExtent() const { return realExtent; }
	
	auto GetOrganizersAspectRatio() const { return organizerAspectRatios.GetRange(); }
	auto GetOrganizersSquares() const { return squaresPerOrganizer.GetRange(); }

	auto GetOrganizers() const { return organizers.begin(); }
	
	//Button& GetButton(const ComponentReference button) { return buttons[button.Component]; }
	
private:
	GTSL::KeepVector<GTSL::KeepVector<uint32, BE::PAR>, BE::PAR> organizersPerOrganizer;
	GTSL::KeepVector<GTSL::KeepVector<Square, BE::PAR>, BE::PAR> squaresPerOrganizer;
	GTSL::KeepVector<uint32, BE::PAR> organizerDepth;
	GTSL::KeepVector<GTSL::Vector2, BE::PAR> organizerAspectRatios;
	GTSL::Vector<uint32, BE::PAR> organizers;
	
	GTSL::Extent2D realExtent;
};

class CanvasSystem : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	ComponentReference CreateCanvas(const Id name)
	{
		return ComponentReference(GetSystemId(), canvases.Emplace());
	}

	Canvas& GetCanvas(const ComponentReference componentReference)
	{
		assertComponentReference(componentReference);
		
		return canvases[componentReference.Component];
	}
	
private:
	GTSL::KeepVector<Canvas, BE::PAR> canvases;
};

class UIManager : public RenderGroup
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	void AddCanvas(const ComponentReference system)
	{
		canvases.Emplace(system);
	}

	auto GetCanvases() { return canvases.GetRange(); }
private:
	GTSL::KeepVector<ComponentReference, BE::PersistentAllocatorReference> canvases;
};
