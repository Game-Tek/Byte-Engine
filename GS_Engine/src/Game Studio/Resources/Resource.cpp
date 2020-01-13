#include "Resource.h"

void ResourceData::Load(InStream& InStream_)
{
	InStream_ >> ResourceName;
}

void ResourceData::Write(OutStream& OutStream_)
{
	OutStream_ << ResourceName;
}
