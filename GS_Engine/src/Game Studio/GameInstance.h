#pragma once

#include "Core.h"
#include "Object.h"

class World;

GS_CLASS GameInstance : public Object
{
public:
	GameInstance();
	~GameInstance();

	void OnUpdate() override;

	void SetActiveWorld(World * NewWorld);

	World * GetWorld() { return ActiveWorld; }

private:
	World * ActiveWorld;
};