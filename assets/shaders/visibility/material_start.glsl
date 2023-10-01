layout(buffer_reference, std430, binding = 0) readonly buffer MaterialStartPassInfo {
	uint entryCount;
	uint treeDepth;
	uint material_count[];
};

layout(buffer_reference, std430, binding = 0) coherent buffer TreeBuffer {
	uint start[];
};

layout(local_size_x = 1) in;
void main() {
	uint sum = 0;
    for (int i = 0; i < entryCount; ++i) {
		start[i] = sum;
		sum += material_count[i];
    }
}