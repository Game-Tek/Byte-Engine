#include "TextResource.h"

#include <fstream>

bool TextResource::LoadResource(const FString& _FullPath)
{
	std::ifstream Input(_FullPath.c_str(), std::ios::in | std::ios::binary);	//Open file as binary

	if (Input.is_open())	//If file is valid
	{
		Input.seekg(0, std::ios::end);	//Search for end
		uint64 FileLength = Input.tellg();		//Get file length
		Input.seekg(0, std::ios::beg);	//Move file pointer back to beginning

		Archive in_archive(&Input);
		
		Data = new TextResourceData;	//Intantiate resource data

		in_archive >> *SCAST(TextResourceData*, Data);
	}
	else
	{
		Input.close();
		return false;
	}

	Input.close();

	return true;
}

void TextResource::LoadFallbackResource(const FString& _FullPath)
{
}
