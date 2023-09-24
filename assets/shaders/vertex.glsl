void main() {
	gl_Position = pc.camera.view_projection * pc.meshes[gl_InstanceIndex].model * vec4(in_position, 1.0);
	out_instance_index = gl_InstanceIndex;
}