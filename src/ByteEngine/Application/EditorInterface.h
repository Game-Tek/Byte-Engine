#pragma once

#include <GTSL/Network/Sockets.h>

#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Game/System.h"

class EditorInterface : public System {
public:
	void Initialize(const InitializeInfo& initializeInfo) override {
		socket.Open(EDITOR_ADDRESS, false);
	}

	void t() {
		GTSL::IPv4Endpoint sender; byte buffer[512];
		auto received = socket.Receive(&sender, GTSL::Range<byte*>(512, buffer));
		if(sender != EDITOR_ADDRESS) {}

		//for every packet
		//	++counter
		//	if counter != packet.index
		//		reject
		
		
		ApplicationManager* gameInstance;
		gameInstance->DispatchEvent("Editor Interface", )
		//push to command list
	}

	void s() {
		auto sendResult = socket.Send(EDITOR_ADDRESS, {});
	}

private:
	static constexpr auto EDITOR_ADDRESS = GTSL::IPv4Endpoint(127, 0, 0, 1, 436);
	GTSL::UDPSocket socket;
	uint16 counter = 0;
};
