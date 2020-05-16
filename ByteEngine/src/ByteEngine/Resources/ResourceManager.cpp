#include "ResourceManager.h"

#include <ostream>
#include <fstream>
#include <GTSL/System.h>
#include "ByteEngine/Debug/Logger.h"

//void ResourceManager::SaveFile(const GTSL::String& _ResourceName, GTSL::String& fileName, ResourceData& ResourceData_)
//{
//	GTSL::String full_path(255, allocatorReference);
//	GTSL::System::GetRunningPath(full_path);
//	full_path += "resources/";
//	full_path += _ResourceName;
//
//	std::ofstream Outfile(full_path.c_str(), std::ios::out | std::ios::binary);
//
//	if (!Outfile.is_open())
//	{
//		BE_LOG_WARNING("Could not save file %s.", _ResourceName.c_str())
//		Outfile.close();
//		return;
//	}
//
//	GTSL::OutStream out_archive(&Outfile);
//
//	Outfile.close();
//}