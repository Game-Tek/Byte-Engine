#pragma once

#include <unordered_map>
#include "Containers/Id.h"
#include "Delegate.h"

class Console
{
	std::unordered_map<Id32, Delegate<void(const FString&)>> commands;
public:
	Console() = default;
	~Console() = default;
	
	void RegisterCommand(const char* name, const Delegate<void(const FString&)>& function) { commands.insert({ name, function }); }
	bool DoCommand(const FString& line)
	{
		
	}
};