#include "Character.h"

#include "Game Studio/Application.h"

#include "Game Studio/Logger.h"

Character::Character()
{
	//GS::Application::GetEventDispatcherInstance()->Subscribe(GS::Application::GetInputManagerInstance()->KeyPressedEventId, this, &reinterpret_cast<MemberFuncPtr>(Character::Move));

	GetGameInstance()->GetWorld()->SetActiveCamera(&MyCamera);
}


Character::~Character()
{
}

void Character::OnUpdate()
{
}

void Character::Move(const KeyPressedEvent & Event)
{
	switch (Event.PressedKey)
	{
	case S:
		AddDeltaPosition(Vector3(0, 0, -10));
	}
}
