#pragma once

#include "RenderableTypeManager.h"

#include "Containers/FVector.hpp"
#include "RAPI/Bindings.h"
#include "RAPI/UniformBuffer.h"

class StaticMeshRenderableManager final : public RenderableTypeManager
{
	struct StaticMeshRenderableData
	{
		Pair<class RAPI::BindingsPool*, RAPI::BindingsSet*> materialData;
	};
	
	FVector<StaticMeshRenderableData> staticMeshRenderablesData;

	Pair<class RAPI::BindingsPool*, RAPI::BindingsSet*> staticMeshesTransformBindings;
	RAPI::UniformBuffer* staticMeshesTransformData = nullptr;
	
public:
	struct StaticMeshRenderableManagerCreateInfo : RenderableTypeManagerCreateInfo
	{
		RAPI::RenderDevice* RenderDevice = nullptr;
	};

	explicit StaticMeshRenderableManager(const StaticMeshRenderableManagerCreateInfo& staticMeshRenderableManagerCreateInfo);
	
	[[nodiscard]] const char* GetName() const override { return "StaticMeshRenderableManager"; }
	
	void DrawObjects(const DrawObjectsInfo& drawObjectsInfo) override;
	void GetRenderableTypeName(FString& name) override;
};
