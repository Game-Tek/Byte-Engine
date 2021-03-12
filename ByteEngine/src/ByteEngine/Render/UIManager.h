#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Extent.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/KeepVector.h>
#include <GTSL/RGB.h>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vector2.h>
#include <GTSL/Tree.hpp>

#include "MaterialSystem.h"
#include "RenderGroup.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Handle.hpp"

MAKE_HANDLE(uint32, Canvas);

enum class Alignment : uint8
{
			TOP,
	LEFT, CENTER, RIGHT,
			BOTTOM
};

enum class ScalingPolicy : uint8
{
	FROM_SCREEN, FROM_OTHER_CONTAINER
};

enum class SizingPolicy : uint8
{
	KEEP_CHILDREN_ASPECT_RATIO,
	SET_ASPECT_RATIO,
	FILL
};

enum class SpacingPolicy : uint8
{
	PACK, DISTRIBUTE
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
	MaterialInstanceHandle Material;
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

//class TexturePrimitive : public Primitive
//{
//public:
//	TexturePrimitive() = default;
//
//	void SetColor(const GTSL::RGBA newColor) { color = newColor; }
//	[[nodiscard]] GTSL::RGBA GetColor() const { return color; }
//	
//	void SetTexture(const ComponentReference newTexture) { textureHandle = newTexture; }
//	
//private:
//	GTSL::RGBA color;
//
//	ComponentReference textureHandle;
//};

class TextPrimitive : public Primitive
{
public:
	TextPrimitive() = default;

	void SetColor(const GTSL::RGBA newColor) { color = newColor; }
	[[nodiscard]] GTSL::RGBA GetColor() const { return color; }
	
	void SetString(const GTSL::Range<const utf8*> newText) { rawString = newText; }
	
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

	void SetSquareMaterial(const uint16 square, const MaterialInstanceHandle material)
	{
		primitives[squares[square].PrimitiveIndex].Material = material;
	}
	
	void SetOrganizerAspectRatio(const uint16 organizer, GTSL::Vector2 aspectRatio)
	{
		primitives[organizersAsPrimitives[organizer]].AspectRatio = aspectRatio;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerAlignment(const uint16 organizer, Alignment alignment)
	{
		organizerAlignments[organizer] = alignment;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	[[nodiscard]] GTSL::Extent2D GetExtent() const { return realExtent; }

	bool CheckHit(GTSL::Vector2 point)
	{
		uint32 i = 0;
		
		for(auto e : organizersAsPrimitives)
		{
			const auto top = (primitives[e].AspectRatio * 0.5f) + primitives[e].RelativeLocation;
			const auto bottom = primitives[e].RelativeLocation - (primitives[e].AspectRatio * 0.5f);
			
			if(point.X() <= top.X() && point.X() >= bottom.X() && point.Y() <= top.Y() && point.Y() >= bottom.Y()) { return true; }

			++i;
		}

		return false;
	}
	
	//[[nodiscard]] auto GetOrganizersAspectRatio() const { return organizerAspectRatios.GetRange(); }

	[[nodiscard]] auto GetOrganizers() const { return organizers.GetRange(); }
	[[nodiscard]] auto& GetOrganizersTree() const { return organizerTree; }
	void SetSquarePosition(uint16 square, GTSL::Vector2 pos)
	{
		BE_ASSERT(pos.X() >= -1.f && pos.X() <= 1.0f && pos.Y() >= -1.0f && pos.Y() <= 1.0f);
		primitives[squares[square].PrimitiveIndex].RelativeLocation = pos;
	}

	auto GetSquares() const { return squares.GetRange(); }
	auto GetPrimitives() const { return primitives.GetRange(); }
	
	void AddSquareToOrganizer(uint16 organizer, uint16 square)
	{
		organizersPrimitives[organizer].EmplaceBack(squares[square].PrimitiveIndex);
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void AddOrganizerToOrganizer(uint16 organizer, uint16 to)
	{
		organizersPerOrganizer[to].EmplaceBack(organizer);
		queueUpdateAndCull(organizer);
	}
	
	void SetOrganizerPosition(uint16 organizer, GTSL::Vector2 pos)
	{
		primitives[organizersAsPrimitives[organizer]].RelativeLocation = pos;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerSizingPolicy(uint16 organizer, SizingPolicy sizingPolicy)
	{
		organizerSizingPolicies[organizer].SizingPolicy = sizingPolicy;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerScalingPolicy(uint16 organizer, ScalingPolicy scalingPolicy)
	{
		organizerSizingPolicies[organizer].ScalingPolicy = scalingPolicy;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerSpacingPolicy(uint16 organizer, SpacingPolicy spacingPolicy)
	{
		organizerSizingPolicies[organizer].SpacingPolicy = spacingPolicy;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void ProcessUpdates();
	
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
	GTSL::KeepVector<GTSL::Vector<uint32, BE::PAR>, BE::PAR> organizersPerOrganizer;
	//GTSL::KeepVector<GTSL::Vector2, BE::PAR> organizerAspectRatios;
	//GTSL::KeepVector<GTSL::Vector2, BE::PAR> organizersPosition;
	GTSL::KeepVector<Alignment, BE::PAR> organizerAlignments;

	struct SizingParameters
	{
		SizingPolicy SizingPolicy;
		ScalingPolicy ScalingPolicy;
		SpacingPolicy SpacingPolicy;
		uint16 OrganizerRef;
	};
	GTSL::KeepVector<SizingParameters, BE::PAR> organizerSizingPolicies;
	
	GTSL::Tree<uint32, BE::PAR> organizerTree;
	
	GTSL::KeepVector<uint32, BE::PAR> organizersAsPrimitives;
	GTSL::KeepVector<decltype(organizerTree)::Node*, BE::PAR> organizers;
	
	GTSL::Extent2D realExtent;

	GTSL::Vector<uint16, BE::PAR> queuedUpdates;

	/**
	 * \brief Queues an organizer update to a list and prunes any redundant children updates if a parent is already updating higher up in the hierarchy.
	 * \param organizer organizer to update from
	 */
	void queueUpdateAndCull(uint32 organizer);
	
	void updateBranch(uint32 organizer);
};

class CanvasSystem : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	CanvasHandle CreateCanvas(const Id name)
	{
		return CanvasHandle(canvases.Emplace());
	}

	Canvas& GetCanvas(const CanvasHandle componentReference)
	{		
		return canvases[componentReference()];
	}
	
	void SignalHit(const GTSL::Vector2 pos)
	{
		for (auto& c : canvases) { if (c.CheckHit(pos)) { BE_LOG_MESSAGE("Hit"); } }
	}

private:
	GTSL::KeepVector<Canvas, BE::PAR> canvases;
};

class UIManager : public RenderGroup
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	void AddCanvas(const CanvasHandle system)
	{
		canvases.Emplace(system);
	}

	auto GetCanvases() { return canvases.GetRange(); }

	void AddColor(const Id name, const GTSL::RGBA color) { colors.Emplace(name, color); }
	[[nodiscard]] GTSL::RGBA GetColor(const Id color) const { return colors.At(color); }

private:
	GTSL::KeepVector<CanvasHandle, BE::PersistentAllocatorReference> canvases;
	GTSL::FlatHashMap<Id, GTSL::RGBA, BE::PAR> colors;
};
