#pragma once

#include <unordered_map>
#include "Containers/Id.h"
#include "Delegate.h"

class CommandMap
{
	std::unordered_map<Id32, Delegate<void(const FString&)>> commands;
public:
	CommandMap() = default;
	~CommandMap() = default;
	
	void RegisterCommand(const char* name, const Delegate<void(const FString&)>& function) { commands.insert({ name, function }); }
	bool DoCommand(const FString& line)
	{
		const auto command_end = line.FindFirst(' ');
		if(command_end == line.npos()) { return false; }

		const Id32 command{ command_end, line.c_str() };
		const auto result = commands.find(command);
		if (result == commands.end()) { return false; }
		commands[command](FString(line.GetLength() - command_end, line, command_end));
		return true;
	}
};