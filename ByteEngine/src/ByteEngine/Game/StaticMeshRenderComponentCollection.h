#pragma once

#include "ComponentCollection.h"

class StaticMeshRenderComponentCollection final : public ComponentCollection
{
public:
	explicit StaticMeshRenderComponentCollection();
	~StaticMeshRenderComponentCollection();

	[[nodiscard]] const char* GetName() const override { return "Static Mesh"; }

	void CreateInstance(const CreateInstanceInfo& createInstanceInfo) override;
	void DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo) override;
	void UpdateInstances(const UpdateInstancesInfo& updateInstancesInfo) override;
	
private:
};
