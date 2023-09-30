layout(local_size_x=1) in;
void main() {
	uint material_index = gl_GlobalInvocationID.x;

	material_count[material_index] += 1;
}