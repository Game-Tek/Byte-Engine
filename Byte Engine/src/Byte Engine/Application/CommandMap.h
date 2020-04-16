#pragma once

#include <unordered_map>
#include <GTSL/Id.h>
#include <GTSL/Delegate.h>
#include <GTSL/String.hpp>

class CommandMap
{
	std::unordered_map<GTSL::Id64::HashType, Delegate<void(const GTSL::String&)>> commands;
public:
	CommandMap() = default;
	~CommandMap() = default;
	
	void RegisterCommand(const char* name, const Delegate<void(const GTSL::String&)>& function) { commands.insert({GTSL::Id64(name), function }); }
	bool DoCommand(const GTSL::String& line)
	{
		const auto command_end = line.FindFirst(' ');
		if(command_end == line.npos()) { return false; }

		const GTSL::Id64 command{ line.c_str() + command_end };
		const auto result = commands.find(command);
		if (result == commands.end()) { return false; }
		//commands[command](GTSL::String(line.GetLength() - command_end, line, command_end));
		return true;
	}
};
