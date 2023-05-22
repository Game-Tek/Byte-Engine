#pragma once

#include "ByteEngine/Object.h"

class World : public Object
{
public:
	World();
	~World();

	struct InitializeInfo
	{
		class ApplicationManager* GameInstance{ nullptr };
	};

	struct DestroyInfo
	{
		class ApplicationManager* GameInstance{ nullptr };
	};

	virtual void InitializeWorld(const InitializeInfo& info);
	virtual void DestroyWorld(const DestroyInfo& destroyInfo);
	virtual void Pause();
	void SetWorldTimeMultiplier(const float multiplier) { m_worldTimeMult = multiplier; }
protected:
	float m_worldTimeMult = 1;
};