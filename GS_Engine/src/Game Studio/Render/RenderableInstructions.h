#pragma once

#include "Utility/Functor.h"
#include "Containers/FString.h"

class RenderComponent;
class Scene;
class Material;
class StaticMesh;

struct CreateInstanceResourcesInfo
{
	RenderComponent* const RenderComponent = nullptr;

	StaticMesh* StaticMesh = nullptr;
	Material* Material = nullptr;
};

struct BuildTypeInstanceSortDataInfo
{
	struct PerInstanceData
	{
		Material* Material = nullptr;
		RenderComponent* const RenderComponent = nullptr;
	};

	FVector<PerInstanceData> InstancesVector;
};

struct BindTypeResourcesInfo
{
	Scene* const Scene = nullptr;
};

struct DrawInstanceInfo
{
	Scene* Scene = nullptr;
	RenderComponent* RenderComponent = nullptr;
};

//Holds a set of functions that describe how to create resources, bind resources for some type of renderable and how to draw it.
struct RenderableInstructions
{
	FString RenderableTypeName = FString("null");

	//This function should create all required data/resources for a single object of the type being described.
	Functor<void (CreateInstanceResourcesInfo&)> CreateInstanceResources;

	//This function should fill out the passed vector to specify all the required parameters for sorting the elements.
	Functor<void (BuildTypeInstanceSortDataInfo&)> BuildTypeInstanceSortData;

	//This function should bind all required resources for the type being described. No per object/instance data.
	Functor<void (BindTypeResourcesInfo&)> BindTypeResources;

	//This function might bind all required resources for the particular instance of the type being rendered and also should draw said instance.
	Functor<void (DrawInstanceInfo&)> DrawInstance;
};