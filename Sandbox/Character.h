#pragma once
#include "Game Studio/WorldObject.h"
#include "Game Studio/KeyPressedEvent.h"
#include "Game Studio/Camera.h"

struct Event;

class Character : public WorldObject
{
public:
	Character();
	~Character();

	void OnUpdate() override;

	void Move(const KeyPressedEvent & Event);

	Camera MyCamera;
};
