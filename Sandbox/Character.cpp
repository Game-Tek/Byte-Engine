#include "Character.h"

#include "Game Studio/Application.h"

#include "Game Studio/Logger.h"
#include "Game Studio/World.h"

#include "Game Studio/EventDispatcher.h"

Character::Character()
{
	//GS::Application::Get()->GetEventDispatcherInstance()->Subscribe(GS::Application::Get()->GetInputManagerInstance()->KeyPressedEventId, this, &Character::Move);

	GetGameInstance()->GetWorld()->SetActiveCamera(&MyCamera);
}

Character::~Character()
{
}

void Character::OnUpdate()
{
	MyCamera.AddDeltaPosition(Vector3(0.0f, 0.0f, 0.005f));
}

void Character::Move(const KeyPressedEvent * Event)
{
	GS_LOG_MESSAGE("Moved!")

	switch (Event->PressedKey)
	{
	default:
		AddDeltaPosition(Vector3());
	case W:
		AddDeltaPosition(Vector3(0, 0, 10));
	case A:
		AddDeltaPosition(Vector3(-10, 0, 0));
	case S:
		AddDeltaPosition(Vector3(0, 0, -10));
	case D:
		AddDeltaPosition(Vector3(10, 0, 0));
	}
}
