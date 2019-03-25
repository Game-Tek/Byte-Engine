#include "Character.h"

#include "Game Studio/Application.h"

#include "Game Studio/Logger.h"
#include "Game Studio/World.h"

Character::Character()
{
	GetGameInstance()->GetWorld()->SetActiveCamera(&MyCamera);
}

Character::~Character()
{
}

void Character::OnUpdate()
{
	MyCamera.AddDeltaPosition(Vector3(0.0f, 0.0001f, -1.0f));
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
