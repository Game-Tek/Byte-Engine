#pragma once

template<typename T>
struct Graph {
private:

	struct Internal	{
		Internal(T d) : data(d) {}

		~Internal() {
			for(uint32 i = 0; i < downstreamCount; ++i) {
				downstream[i] = nullptr;
			}

			downstreamCount = 0u;
			upstreamCount = 0u;
		}

		T data;
		Internal* downstream[64] = { nullptr };
		Internal* upstream[64] = { nullptr };
		uint32 downstreamCount = 0, upstreamCount = 0;
	}* internal = nullptr;

	bool shared = false;

public:
	explicit Graph(T d) : internal(new Internal(d)) {}

	Graph(Internal* internal) : internal(internal), shared(true) {}

	Graph(Graph&& other) noexcept : internal(other.internal) {
		other.internal = nullptr;
	}

	Graph(const Graph& other) : internal(new Internal(other.GetData())) {
		internal->downstreamCount = other.internal->downstreamCount;
		internal->upstreamCount = other.internal->upstreamCount;

		for(uint32 i = 0; i < internal->downstreamCount; ++i) {
			internal->downstream[i] = other.internal->downstream[i];
		}

		for(uint32 i = 0; i < internal->upstreamCount; ++i) {
			internal->upstream[i] = other.internal->upstream[i];
		}
	}

	~Graph() {
		if(internal && !shared) { delete internal; }
		internal = nullptr;
	}

	void Connect(Graph& other) {
		for(uint32 i = 0; i < other.internal->upstreamCount; ++i) {
			if(other.internal->upstream[i] == other.internal) {
				BE_DEBUG_BREAK;
			}
		}

		internal->downstream[internal->downstreamCount++] = other.internal;
		other.internal->upstream[other.internal->upstreamCount++] = internal;
	}

	auto GetParents() const {
		GTSL::StaticVector<Graph, 64> children;

		for(uint32 i = 0; i < internal->upstreamCount; ++i) {
			children.EmplaceBack(internal->upstream[i]);
		}

		return children;
	}

	auto GetChildren() const {
		GTSL::StaticVector<Graph, 64> children;

		for(uint32 i = 0; i < internal->downstreamCount; ++i) {
			children.EmplaceBack(internal->downstream[i]);
		}

		return children;
	}

	T& GetData() { return internal->data; }
	const T& GetData() const { return internal->data; }
};