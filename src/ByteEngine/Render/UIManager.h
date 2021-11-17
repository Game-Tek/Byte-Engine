#pragma once

#include "ByteEngine/Game/System.h"

#include <GTSL/Extent.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/FixedVector.hpp>
#include <GTSL/RGB.h>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vectors.h>
#include <GTSL/Tree.hpp>

#include "RenderTypes.h"
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

class Canvas : public Object
{
public:
	Canvas();

	void SetExtent(const GTSL::Extent2D newExtent) { realExtent = newExtent; }

	uint16 AddOrganizer(const Id organizerName);
	uint16 AddOrganizer(const Id organizerName, const uint16 parentOrganizer);

	uint16 AddSquare() {
		const auto primitiveIndex = primitives.Emplace();
		const auto place = squares.Emplace();
		squares[place].PrimitiveIndex = primitiveIndex;
		auto& primitive = primitives[primitiveIndex];
		primitive.AspectRatio = 1.f;
		return static_cast<uint16>(place);
	}

	void SetSquareAspectRatio(const uint16 square, const GTSL::Vector2 aspectRatio) {
		primitives[squares[square].PrimitiveIndex].AspectRatio = aspectRatio;
	}

	void SetSquareColor(const uint16 square, const Id color) {
		squares[square].SetColor(color);
	}

	void SetSquareMaterial(const uint16 square, const MaterialInstanceHandle material) {
		primitives[squares[square].PrimitiveIndex].Material = material;
	}
	
	void SetOrganizerAspectRatio(const uint16 organizer, GTSL::Vector2 aspectRatio) {
		primitives[organizersAsPrimitives[organizer]].AspectRatio = aspectRatio;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerAlignment(const uint16 organizer, Alignment alignment) {
		organizerAlignments[organizer] = alignment;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	[[nodiscard]] GTSL::Extent2D GetExtent() const { return realExtent; }

	bool CheckHit(GTSL::Vector2 point) {
		uint32 i = 0;
		
		for(auto e : organizersAsPrimitives) {
			const auto top = (primitives[e].AspectRatio * 0.5f) + primitives[e].RelativeLocation;
			const auto bottom = primitives[e].RelativeLocation - (primitives[e].AspectRatio * 0.5f);
			
			if(point.X() <= top.X() && point.X() >= bottom.X() && point.Y() <= top.Y() && point.Y() >= bottom.Y()) { return true; }

			++i;
		}

		return false;
	}
	
	//[[nodiscard]] auto GetOrganizersAspectRatio() const { return organizerAspectRatios.GetRange(); }

	//[[nodiscard]] auto& GetOrganizers() const { return organizers; }
	//[[nodiscard]] auto& GetOrganizersTree() const { return organizerTree; }
	void SetSquarePosition(uint16 square, GTSL::Vector2 pos) {
		BE_ASSERT(pos.X() >= -1.f && pos.X() <= 1.0f && pos.Y() >= -1.0f && pos.Y() <= 1.0f);
		primitives[squares[square].PrimitiveIndex].RelativeLocation = pos;
	}

	auto& GetSquares() const { return squares; }
	auto& GetPrimitives() const { return primitives; }
	
	void AddSquareToOrganizer(uint16 organizer, uint16 square) {
		organizersPrimitives[organizer].EmplaceBack(squares[square].PrimitiveIndex);
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void AddOrganizerToOrganizer(uint16 organizer, uint16 to) {
		organizersPerOrganizer[to].EmplaceBack(organizer);
		queueUpdateAndCull(organizer);
	}
	
	void SetOrganizerPosition(uint16 organizer, GTSL::Vector2 pos) {
		primitives[organizersAsPrimitives[organizer]].RelativeLocation = pos;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerSizingPolicy(uint16 organizer, SizingPolicy sizingPolicy) {
		organizerSizingPolicies[organizer].SizingPolicy = sizingPolicy;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerScalingPolicy(uint16 organizer, ScalingPolicy scalingPolicy) {
		organizerSizingPolicies[organizer].ScalingPolicy = scalingPolicy;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void SetOrganizerSpacingPolicy(uint16 organizer, SpacingPolicy spacingPolicy) {
		organizerSizingPolicies[organizer].SpacingPolicy = spacingPolicy;
		updateBranch(organizer);
		queueUpdateAndCull(organizer);
	}

	void ProcessUpdates();
	
private:
	GTSL::FixedVector<PrimitiveData, BE::PAR> primitives;
	GTSL::FixedVector<Square, BE::PAR> squares;
	GTSL::FixedVector<uint32, BE::PAR> organizerDepth;
	GTSL::FixedVector<GTSL::Vector<uint32, BE::PAR>, BE::PAR> organizersPrimitives;
	GTSL::FixedVector<GTSL::Vector<uint32, BE::PAR>, BE::PAR> organizersPerOrganizer;
	GTSL::FixedVector<Alignment, BE::PAR> organizerAlignments;

	struct SizingParameters {
		SizingPolicy SizingPolicy;
		ScalingPolicy ScalingPolicy;
		SpacingPolicy SpacingPolicy;
		uint16 OrganizerRef;
	};
	GTSL::FixedVector<SizingParameters, BE::PAR> organizerSizingPolicies;
	
	//GTSL::Tree<uint32, BE::PAR> organizerTree;
	
	GTSL::FixedVector<uint32, BE::PAR> organizersAsPrimitives;
	//GTSL::FixedVector<decltype(organizerTree)::Node*, BE::PAR> organizers;
	
	GTSL::Extent2D realExtent;

	GTSL::Vector<uint16, BE::PAR> queuedUpdates;

	/**
	 * \brief Queues an organizer update to a list and prunes any redundant children updates if a parent is already updating higher up in the hierarchy.
	 * \param organizer organizer to update from
	 */
	void queueUpdateAndCull(uint32 organizer);
	
	void updateBranch(uint32 organizer);
};

MAKE_HANDLE(uint32, Organizer)
MAKE_HANDLE(uint32, Square)

class CanvasSystem : public System
{
public:
	CanvasSystem(const InitializeInfo& initializeInfo);
	
	CanvasHandle CreateCanvas(const Id) {
		return CanvasHandle(canvases.Emplace());
	}
	
	void SignalHit(const GTSL::Vector2 pos)
	{
		for (auto& c : canvases) { if (c.CheckHit(pos)) { BE_LOG_MESSAGE(u8"Hit"); } }
	}

	void SetExtent(CanvasHandle canvasHandle, GTSL::Extent2D extent) {
		canvases[canvasHandle()].SetExtent(extent);
	}

	OrganizerHandle AddOrganizer(CanvasHandle canvasHandle, Id organizerName) {
		canvases[canvasHandle()].AddOrganizer(organizerName);
		return OrganizerHandle();
	}

	SquareHandle AddSquare() {}

	void SetColor(SquareHandle squareHandle, Id colorName);
	void SetMaterial(SquareHandle squareHandle, MaterialInstanceHandle materialInstanceHandle);
	void AddToOrganizer(OrganizerHandle organizerHandle, SquareHandle squareHandle);
	void SetAspectRatio(OrganizerHandle organizerHandle, GTSL::Vector2 extent);
	void SetAlignment(OrganizerHandle organizerHandle, Alignment alignment);
	void SetPosition(OrganizerHandle organizerHandle, GTSL::Vector2 position);
	void SetSizingPolicy(OrganizerHandle organizerHandle, SizingPolicy sizingPolicy);
	void SetScalingPolicy(OrganizerHandle organizerHandle, ScalingPolicy scalingPolicy);
	void SetSpacingPolicy(OrganizerHandle organizerHandle, SpacingPolicy spacingPolicy);

private:
	GTSL::FixedVector<Canvas, BE::PAR> canvases;
};

class UIManager : public System
{
public:
	explicit UIManager(const InitializeInfo& initializeInfo);

	void AddCanvas(const CanvasHandle system)
	{
		canvases.Emplace(system);
	}

	auto& GetCanvases() { return canvases; }

	void AddColor(const Id colorName, const GTSL::RGBA color) { colors.Emplace(colorName, color); }
	[[nodiscard]] GTSL::RGBA GetColor(const Id color) const { return colors.At(color); }

private:
	GTSL::FixedVector<CanvasHandle, BE::PersistentAllocatorReference> canvases;
	GTSL::HashMap<Id, GTSL::RGBA, BE::PAR> colors;
};
