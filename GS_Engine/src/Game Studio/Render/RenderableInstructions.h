#pragma once

#include "Utility/Functor.h"
#include "Containers/FString.h"

//Holds a set of functions that describe how to create resources, bind resources for some type of renderable and how to draw it.
struct RenderableInstructions
{
	FString RenderableTypeName = FString("null");

	//This function should create all required data/resources for a single object of the type being described.
	Functor<void ()> CreateInstanceResources;

	//This function should bind all required resources for the type being described. No per object/instance data.
	Functor<void ()> BindTypeResources;

	//This function might bind all required resources for the particular instance of the type being rendered and also should draw said instance.
	Functor<void ()> DrawInstance;
};