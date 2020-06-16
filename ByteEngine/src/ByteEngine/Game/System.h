#pragma once

#include <GTSL/Ranger.h>

#include "ByteEngine/Object.h"

/**
 * \brief Systems persist across levels and can process world components regardless of the current level.
 * Used to instantiate render engines, sound engines, physics engines, AI systems, etc.
 */
class System : public Object
{
public:
	virtual void Initialize() = 0;
	
	virtual void Process(const GTSL::Ranger<class World*>& worlds) = 0;

	virtual void Shutdown() = 0;

	[[nodiscard]] const char* GetName() const override { return "System"; }
private:
};
