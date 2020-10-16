#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Extent.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/KeepVector.h>
#include <GTSL/RGB.h>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vector2.h>
#include <GTSl/Tree.hpp>

#include "RenderGroup.h"
#include "ByteEngine/Id.h"

enum class Alignment : uint8
{
	LEFT, CENTER, RIGHT
};

enum class SizingPolicy : uint8
{
	FROM_WINDOW, FROM_OTHER_CONTAINER
};

class Button : public Object
{
public:

	void SetMaterial(const ComponentReference newMat) { material = newMat; }
	
private:
	ComponentReference material;
};

struct PrimitiveData
{
	GTSL::Vector2 RelativeLocation;
	GTSL::Vector2 AspectRatio;
	Alignment Alignment;
	SizingPolicy SizingPolicy;
};

struct Primitive
{
	uint32 PrimitiveIndex;
};

class Square : public Primitive
{
public:
	Square() = default;
	
	void SetColor(const Id newColor) { color = newColor; }
	[[nodiscard]] Id GetColor() const { return color; }
	
private:
	Id color;
	float32 rotation = 0.0f;
};

class TexturePrimitive : public Primitive
{
public:
	TexturePrimitive() = default;

	void SetColor(const GTSL::RGBA newColor) { color = newColor; }
	[[nodiscard]] GTSL::RGBA GetColor() const { return color; }
	
	void SetTexture(const ComponentReference newTexture) { textureHandle = newTexture; }
	
private:
	GTSL::RGBA color;

	ComponentReference textureHandle;
};

class TextPrimitive : public Primitive
{
public:
	TextPrimitive() = default;

	void SetColor(const GTSL::RGBA newColor) { color = newColor; }
	[[nodiscard]] GTSL::RGBA GetColor() const { return color; }
	
	void SetString(const GTSL::Range<const UTF8*> newText) { rawString = newText; }
	
private:	
	GTSL::RGBA color;
	
	GTSL::String<BE::PAR> rawString;
};

class Canvas : public Object
{
public:
	Canvas();

	void SetExtent(const GTSL::Extent2D newExtent) { realExtent = newExtent; }

	uint16 AddOrganizer(const Id name);
	uint16 AddOrganizer(const Id name, const uint16 parentOrganizer);

	uint16 AddSquare(const uint16 organizer)
	{
		return squaresPerOrganizer[organizer].Emplace();
	}

	void SetSquareAspectRatio(const uint16 organizer, const uint16 square, const GTSL::Vector2 aspectRatio)
	{
		primitivesPerOrganizer[organizer][squaresPerOrganizer[organizer][square].PrimitiveIndex].AspectRatio = aspectRatio;
	}

	void SetSquareColor(const uint16 organizer, const uint16 square, const Id color)
	{
		squaresPerOrganizer[organizer][square].SetColor(color);
	}

	uint16 AddButton(const ComponentReference organizer, const Id name);
	
	void SetOrganizerAspectRatio(const uint16 organizer, GTSL::Vector2 aspectRatio)
	{
		organizerAspectRatios[organizer] = aspectRatio;
	}

	void SetOrganizerAlignment(const uint16 organizer, Alignment alignment)
	{
		organizerAlignments[organizer] = alignment;
	}

	[[nodiscard]] GTSL::Extent2D GetExtent() const { return realExtent; }

	[[nodiscard]] auto GetOrganizersAspectRatio() const { return organizerAspectRatios.GetRange(); }
	[[nodiscard]] auto GetOrganizersSquares() const { return squaresPerOrganizer.GetRange(); }

	[[nodiscard]] auto GetOrganizers() const { return organizers.GetRange(); }
	[[nodiscard]] auto& GetOrganizersTree() const { return organizerTree; }

	auto GetPrimitivesPerOrganizer() const
	{
		return primitivesPerOrganizer.begin();
	}
	
	//Button& GetButton(const ComponentReference button) { return buttons[button.Component]; }
	
private:
	GTSL::KeepVector<GTSL::KeepVector<PrimitiveData, BE::PAR>, BE::PAR> primitivesPerOrganizer;
	GTSL::KeepVector<GTSL::KeepVector<Square, BE::PAR>, BE::PAR> squaresPerOrganizer;
	GTSL::KeepVector<uint32, BE::PAR> organizerDepth;
	GTSL::KeepVector<GTSL::Vector2, BE::PAR> organizerAspectRatios;
	GTSL::KeepVector<Alignment, BE::PAR> organizerAlignments;
	GTSL::KeepVector<SizingPolicy, BE::PAR> organizerSizingPolicies;	
	
	GTSL::Tree<uint32, BE::PAR> organizerTree;
	
	GTSL::KeepVector<decltype(organizerTree)::Node*, BE::PAR> organizers;
	
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

	void AddColor(const Id name, const GTSL::RGBA color) { colors.Emplace(name, color); }
	[[nodiscard]] GTSL::RGBA GetColor(const Id color) const { return colors.At(color); }
private:
	GTSL::KeepVector<ComponentReference, BE::PersistentAllocatorReference> canvases;
	GTSL::FlatHashMap<GTSL::RGBA, BE::PAR> colors;
};
