#version 450 core
#pragma shader_stage(fragment)

layout(location = 0) out vec4 out_color;

void main() {
	out_color = vec4(vec3(gl_FragCoord.xy, 0.5), 1.0);
}