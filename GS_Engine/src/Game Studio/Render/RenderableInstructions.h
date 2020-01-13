#pragma once

#include "Utility/Functor.h"
#include "Containers/FString.h"

class RenderComponent;
class Renderer;
class Material;
class StaticMesh;

struct RenderComponentCreateInfo;


/**
 * \brief Holds information to specify how to create an instance of a render component.
 */
struct CreateInstanceResourcesInfo
{
	
	/**
	 * \brief Pointer to the render component being created.
	 */
	RenderComponent* const RenderComponent = nullptr;
	
	/**
	 * \brief Pointer to the scene creating the render component.
	 */
	Renderer* Scene = nullptr;

	/**
	 * \brief Pointer to a RenderComponentCreateInfo which contains information specified during construction for how to instantiate this RenderComponent.
	 */
	RenderComponentCreateInfo* RenderComponentCreateInfo = nullptr;
	
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
	Renderer* const Scene = nullptr;
};


/**
 * \brief Holds information to specify the render component to be rendered. 
 */
struct DrawInstanceInfo
{
	
	/**
	 * \brief Pointer to the scene rendering the render component.
	 */
	Renderer* Scene = nullptr;
	
	/**
	 * \brief Pointer to the render component to be rendered.
	 */
	RenderComponent* RenderComponent = nullptr;
};

//Holds a set of functions that describe how to create resources, bind resources for some type of renderable and how to draw it.
struct RenderableInstructions
{
	RenderableInstructions() = default;
	
	//FString RenderableTypeName = FString("null");

	//This function should create all required data/resources for a single object of the type being described.
	Functor<void (CreateInstanceResourcesInfo&)> CreateInstanceResources;

	//This function should fill out the passed vector to specify all the required parameters for sorting the elements.
	Functor<void (BuildTypeInstanceSortDataInfo&)> BuildTypeInstanceSortData;

	//This function should bind all required resources for the type being described. No per object/instance data.
	Functor<void (BindTypeResourcesInfo&)> BindTypeResources;

	//This function might bind all required resources for the particular instance of the type being rendered and also should draw said instance.
	Functor<void (DrawInstanceInfo&)> DrawInstance;
};