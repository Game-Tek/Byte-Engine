#include "UIManager.h"

Canvas::Canvas() : Object("Canvas"), organizers(4, GetPersistentAllocator()), organizerDepth(4, GetPersistentAllocator()), organizerAspectRatios(4, GetPersistentAllocator()), squares(8, GetPersistentAllocator()),
primitives(8, GetPersistentAllocator()), organizersPrimitives(4, GetPersistentAllocator()), organizersPosition(4, GetPersistentAllocator()), organizerSizingPolicies(4, GetPersistentAllocator()),
organizerAlignments(4, GetPersistentAllocator())
{
	organizerTree.Initialize(GetPersistentAllocator());
}

uint16 Canvas::AddOrganizer(const Id name)
{
	auto organizer = organizerDepth.Emplace(0);
	organizerAspectRatios.Emplace();
	organizerAlignments.Emplace();
	organizerSizingPolicies.Emplace();
	organizerDepth.Emplace();
	organizersPosition.Emplace();
	organizersPrimitives.Emplace(4, GetPersistentAllocator());

	auto node = organizerTree.GetRootNode();
	node->Data = organizer;
	
	organizers.EmplaceAt(organizer, node);

	return organizer;
}

uint16 Canvas::AddOrganizer(const Id name, const uint16 parentOrganizer)
{
	auto organizer = organizerDepth.Emplace(0);
	organizerAspectRatios.Emplace();
	organizerAlignments.Emplace();
	organizerSizingPolicies.Emplace();
	organizerDepth.Emplace();
	organizersPosition.Emplace();
	organizersPrimitives.Emplace(4, GetPersistentAllocator());
	//squaresPerOrganizer.Emplace(4, GetPersistentAllocator());
	
	auto* child = organizerTree.AddChild(organizers[parentOrganizer]);
	child->Data = organizer;
	
	organizers.EmplaceAt(organizer, child);

	return organizer;
}

void CanvasSystem::Initialize(const InitializeInfo& initializeInfo)
{
	canvases.Initialize(8, GetPersistentAllocator());
}

void CanvasSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

void UIManager::Initialize(const InitializeInfo& initializeInfo)
{
	canvases.Initialize(8, GetPersistentAllocator());
	colors.Initialize(16, GetPersistentAllocator());
}

void UIManager::Shutdown(const ShutdownInfo& shutdownInfo)
{
}