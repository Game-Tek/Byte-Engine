#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/Id.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/String.hpp>

class CommandMap : public Object
{
public:
	CommandMap() = default;
	~CommandMap() = default;
	
	void RegisterCommand(const char* name, const GTSL::Delegate<void(const GTSL::String&)>& function) { commands.Emplace(GetPersistentAllocator(), GTSL::Id64(name), function); }
	bool DoCommand(const GTSL::String& line)
	{
		const auto command_end = line.FindFirst(' ');
		if(command_end == line.npos()) { return false; }

		const GTSL::Id64 command{ line.c_str() + command_end };
		const auto result = commands.Find(command);
		if (result == commands.end()) { return false; }
		//commands[command](GTSL::String(line.GetLength() - command_end, line, command_end));
		return true;
	}

private:
	GTSL::FlatHashMap<Id, GTSL::Delegate<void(const GTSL::String&)>> commands;
};
