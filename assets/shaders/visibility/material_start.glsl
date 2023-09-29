// All threads in the same group will share the same atomic result.
shared uint atomic;

#define WAVE_SIZE 32
#define WAVE_SIZE_BASE uint(log2(WAVE_SIZE))

layout(buffer_reference, std430, binding = 0) readonly buffer MaterialStartPassInfo {
	uint entryCount;
	uint treeDepth;
	uint treeOffsets[];
};

layout(buffer_reference, std430, binding = 0) coherent buffer TreeBuffer {
	uint tree[];
};

layout(local_size_x = WAVE_SIZE) in;
void main() {
	uint groupId = gl_WorkGroupID.x;
	uint localThreadId = gl_LocalInvocationIndex;
    
    // A buffer of atomics initialized to 0. 
    atomicsBuffer = [];

    for (int i = 0; i < treeDepth; ++i) {
        // The global thread ID is unique for all threads launched and corresponds to two elements in the tree layer below.
        uint globalThreadId = (groupId << WAVE_SIZE_BASE) + localThreadId;
        uint readOffset = treeOffsets[i];
        uint writeOffset = treeOffsets[i + 1];

        // This thread is only valid if the tree has one or two elements in the layer below that this thread should sum up.
        bool validThread = globalThreadId < materialStartPassInfo.entryCount;

        if (validThread) {
            // Sum the two elements in the previous layer and write them for the current layer.
            uint elementReadOffset = readOffset + (globalThreadId * 2);
            uint elementWriteOffset = readOffset + globalThreadId;
            tree[elementWriteOffset] = tree[elementReadOffset + 0] + tree[elementReadOffset + 1];
        }

		uint weight = 0;

		if (validThread) {
			weight = tree[treeOffsets[i] + globalThreadId];
		}

		uint sum = subgroupExclusiveAdd(weight) + weight;

		// The last thread in the wave will write the sum.
        if (localThreadId == WAVE_SIZE - 1) {
			tree[treeOffsets[i + 1] + groupId] = sum;
		}

        // Sync to ensure that the atomic for this thread group has been updated and will be visible for all threads in the group.
        groupMemoryBarrier(); barrier(); memoryBarrier();

        // Two thread groups wrote to the same atomic, only the last one to write survives for the next tree layer.
        if (atomic < 1) {
            return;
        }

        // Sync to make sure that all threads within a group have finished reading the value of `atomic`.
        // This is needed because it will be modified again in the next iteration.
		groupMemoryBarrier(); barrier();
    }
}