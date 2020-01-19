#include "TextResource.h"

#include <fstream>

bool TextResource::loadResource(const LoadResourceData& LRD_)
{
	std::ifstream Input(LRD_.FullPath.c_str(), std::ios::in | std::ios::binary);	//Open file as binary

	if (Input.is_open())	//If file is valid
	{
		Input.seekg(0, std::ios::end);	//Search for end
		uint64 FileLength = Input.tellg();		//Get file length
		Input.seekg(0, std::ios::beg);	//Move file pointer back to beginning

		InStream in_archive(&Input);

		in_archive >> data;
	}
	else
	{
		Input.close();
		return false;
	}

	Input.close();

	return true;
}

void TextResource::loadFallbackResource(const FString& _FullPath)
{
}
