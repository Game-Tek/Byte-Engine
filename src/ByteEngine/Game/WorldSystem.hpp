#pragma once

#include "ByteEngine/Game/System.hpp"

#include <GTSL/File.hpp>

#include "ByteEngine/Render/LightsRenderGroup.h"

class WorldSystem : public BE::System {
	static void SetPosition(const GTSL::JSON<BE::TAR>& json, auto system, auto handle) {
		auto jsonPosition = json[u8"pos"];
		auto pos = GTSL::Vector3(jsonPosition[0].GetFloat(), jsonPosition[1].GetFloat(), jsonPosition[2].GetFloat());
		system->SetPosition(handle, pos);
	}

	static void SetRotation(const GTSL::JSON<BE::TAR>& json, auto system, auto handle) {
		auto jsonRotation = json[u8"rot"];
		if(!jsonRotation) { return; }
		auto rot = GTSL::Rotator(GTSL::Math::DegreesToRadians(jsonRotation[0].GetFloat()), GTSL::Math::DegreesToRadians(jsonRotation[1].GetFloat()), GTSL::Math::DegreesToRadians(jsonRotation[2].GetFloat()));
		system->SetRotation(handle, GTSL::Quaternion(rot));
	}

	static void SetColor(const GTSL::JSON<BE::TAR>& json, auto system, auto handle) {
		auto jsonColor = json[u8"color"];
		auto color = GTSL::RGBA(jsonColor[0].GetFloat(), jsonColor[1].GetFloat(), jsonColor[2].GetFloat(), jsonColor[3].GetFloat());
		system->SetColor(handle, GTSL::RGB(color.R(), color.G(), color.B()));
	}
public:
	WorldSystem(const InitializeInfo& initialize_info) : System(initialize_info, u8"WorldSystem") {
		GTSL::File file(ResourceManager::GetUserResourcePath(u8"level.json"));

		GTSL::StaticBuffer<8192> fileBuffer(file);

		auto json = GTSL::JSON(GTSL::StringView(fileBuffer), GetTransientAllocator());
		
		auto worldName = json[u8"name"].GetStringView();

		auto* staticMeshSystem = GetApplicationManager()->GetSystem<StaticMeshRenderGroup>(u8"StaticMeshRenderGroup");
		auto lightsSystem = GetApplicationManager()->GetSystem<LightsRenderGroup>(u8"LightsRenderGroup");

		for(auto e : json[u8"elements"]) {
			if(auto m = e[u8"type"]; m.GetStringView() == u8"Mesh") {
				auto componentName = e[u8"name"];
				auto resourceName = e[u8"mesh"];

				auto staticMeshHandle = staticMeshSystem->AddStaticMesh(Id(resourceName));

				SetPosition(e, staticMeshSystem, staticMeshHandle);
				SetRotation(e, staticMeshSystem, staticMeshHandle);
			}

			if(auto m = e[u8"type"]; m.GetStringView() == u8"Light") {
				auto componentName = e[u8"name"];

				auto lightHandle = lightsSystem->CreatePointLight();

				SetPosition(e, lightsSystem, lightHandle);
				SetColor(e, lightsSystem, lightHandle);
			}
		}
	}
};
