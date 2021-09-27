#pragma once

#include <GTSL/Tree.hpp>
#include <GTSL/Math/Vectors.h>
#include <GTSL/Math/Math.hpp>

struct GraphNode {
	GTSL::Vector3 Position;
};

void AStar() {
	using TreeType = GTSL::Tree<GraphNode, GTSL::DefaultAllocatorReference>;
	TreeType tree;

	auto advanceNode = [&](TreeType::Node* node, auto&& self) -> void {
		TreeType::Node* shortestNode = nullptr;

		GTSL::Vector3 currentPosition;
		float32 shortestLength = FLT_MAX;

		for(auto e : node->Nodes) {
			if (auto len = GTSL::SquaredDistance(e->Position, currentPosition); len < shortestLength) {
				shortestLength = len;
				shortestNode = e;
			}
		}

		self(shortestNode, self);
	};

	advanceNode(&tree[0], advanceNode);
}