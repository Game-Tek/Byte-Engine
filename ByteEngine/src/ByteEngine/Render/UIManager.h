#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Extent.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/KeepVector.h>
#include <GTSL/RGB.h>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vector2.h>
#include <GTSl/Tree.hpp>


#include "MaterialSystem.h"
#include "RenderGroup.h"
#include "ByteEngine/Id.h"

enum class Alignment : uint8
{
			TOP,
	LEFT, CENTER, RIGHT,
			BOTTOM
};

enum class SizingPolicy : uint8
{
	FROM_WINDOW, FROM_OTHER_CONTAINER
};

class Button : public Object
{
public:
	
private:
};

struct PrimitiveData
{
	GTSL::Vector2 RelativeLocation;
	GTSL::Vector2 AspectRatio;
	Alignment Alignment;
	SizingPolicy SizingPolicy;
	MaterialHandle Material;
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

	uint16 AddSquare()
	{
		const auto primitiveIndex = primitives.Emplace();
		const auto place = squares.Emplace();
		squares[place].PrimitiveIndex = primitiveIndex;
		return static_cast<uint16>(place);
	}

	void SetSquareAspectRatio(const uint16 square, const GTSL::Vector2 aspectRatio)
	{
		primitives[squares[square].PrimitiveIndex].AspectRatio = aspectRatio;
	}

	void SetSquareColor(const uint16 square, const Id color)
	{
		squares[square].SetColor(color);
	}

	void SetSquareMaterial(const uint16 square, const MaterialHandle material)
	{
		primitives[squares[square].PrimitiveIndex].Material = material;
	}
	
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

	[[nodiscard]] auto GetOrganizers() const { return organizers.GetRange(); }
	[[nodiscard]] auto& GetOrganizersTree() const { return organizerTree; }
	void SetSquarePosition(uint16 square, GTSL::Vector2 pos)
	{
		BE_ASSERT(pos.X >= -1.f && pos.X <= 1.0f && pos.Y >= -1.0f && pos.Y <= 1.0f);
		primitives[squares[square].PrimitiveIndex].RelativeLocation = pos;
	}

	auto GetSquares() const { return squares.GetRange(); }
	auto GetPrimitives() const { return primitives.GetRange(); }
	
	void AddSquareToOrganizer(uint16 organizer, uint16 square)
	{
		organizersPrimitives[organizer].EmplaceBack(squares[square].PrimitiveIndex);
		auto& primitive = primitives[squares[square].PrimitiveIndex];

		auto orgAR = organizerAspectRatios[organizer];
		auto orgLoc = organizersPosition[organizer];

		GTSL::Vector2 perPrimitiveInOrganizerAspectRatio;

		switch (organizerAlignments[organizer])
		{
		case Alignment::LEFT: break;
		case Alignment::CENTER: break;
		case Alignment::RIGHT: break;
		default: break;
		}

		perPrimitiveInOrganizerAspectRatio.X = orgAR.X / organizersPrimitives[organizer].GetLength();
		perPrimitiveInOrganizerAspectRatio.Y = orgAR.Y;

		GTSL::Vector2 startPos = { (-(orgAR.X / 2.0f) + (perPrimitiveInOrganizerAspectRatio.X / 2)), orgLoc.Y };
		GTSL::Vector2 increment = perPrimitiveInOrganizerAspectRatio;
		
		for (uint32 i = 0; i < organizersPrimitives[organizer].GetLength(); ++i)
		{
			primitives[organizersPrimitives[organizer][i]].AspectRatio = perPrimitiveInOrganizerAspectRatio;
			primitives[organizersPrimitives[organizer][i]].RelativeLocation = startPos;

			startPos += increment;
		}
	}

	void SetOrganizerPosition(uint16 organizerComp, GTSL::Vector2 pos)
	{
		organizersPosition[organizerComp] = pos;
	}

	//auto GetPrimitivesPerOrganizer() const
	//{
	//	return primitivesPerOrganizer.begin();
	//}
	
	//Button& GetButton(const ComponentReference button) { return buttons[button.Component]; }
	
private:
	//GTSL::KeepVector<GTSL::KeepVector<PrimitiveData, BE::PAR>, BE::PAR> primitivesPerOrganizer;
	GTSL::KeepVector<PrimitiveData, BE::PAR> primitives;
	GTSL::KeepVector<Square, BE::PAR> squares;
	GTSL::KeepVector<uint32, BE::PAR> organizerDepth;
	GTSL::KeepVector<GTSL::Vector<uint32, BE::PAR>, BE::PAR> organizersPrimitives;
	GTSL::KeepVector<GTSL::Vector2, BE::PAR> organizerAspectRatios;
	GTSL::KeepVector<GTSL::Vector2, BE::PAR> organizersPosition;
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
