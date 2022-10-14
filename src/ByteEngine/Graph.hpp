#pragma once

template<typename T>
struct Graph {
private:

	struct Internal	{
		Internal(T d) : data(d) {}

		~Internal() {
			for(uint32 i = 0; i < childrenCount; ++i) {
				nodes[i] = nullptr;
			}

			childrenCount = 0u;
		}

		T data;
		Internal* nodes[64] = { nullptr };
		uint32 childrenCount = 0;
	}* internal = nullptr;

	bool shared = false;

public:
	explicit Graph(T d) : internal(new Internal(d)) {}

	Graph(Internal* internal) : internal(internal), shared(true) {}

	Graph(Graph&& other) noexcept : internal(other.internal) {
		other.internal = nullptr;
	}

	Graph(const Graph& other) : internal(new Internal(other.GetData())) {
		internal->childrenCount = other.internal->childrenCount;

		for(uint32 i = 0; i < internal->childrenCount; ++i) {
			internal->nodes[i] = other.internal->nodes[i];
		}
	}

	~Graph() {
		if(internal && !shared) { delete internal; }
		internal = nullptr;
	}

	void Connect(const Graph& other) {
		internal->nodes[internal->childrenCount++] = other.internal;
	}

	auto GetChildren() const {
		GTSL::StaticVector<Graph, 64> children;

		for(uint32 i = 0; i < internal->childrenCount; ++i) {
			children.EmplaceBack(internal->nodes[i]);
		}

		return children;
	}

	T& GetData() { return internal->data; }
	const T& GetData() const { return internal->data; }
};