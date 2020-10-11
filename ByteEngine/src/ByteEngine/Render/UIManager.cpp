#include "UIManager.h"

Canvas::Canvas() : Object("Canvas")
{
}

uint16 Canvas::AddOrganizer(const Id name)
{
	organizerDepth.Emplace(0);
	organizerAspectRatios.Emplace();
	organizersPerOrganizer.Emplace(4, GetPersistentAllocator());
	squaresPerOrganizer.Emplace(4, GetPersistentAllocator());
}

uint16 Canvas::AddButton(const ComponentReference organizer, const Id name, const Alignment alignment)
{
	return squaresPerOrganizer[organizer.Component].Emplace();
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
}

void UIManager::Shutdown(const ShutdownInfo& shutdownInfo)
{
}