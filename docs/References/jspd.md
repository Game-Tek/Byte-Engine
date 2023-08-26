---
icon: file-code
---

# JSON Shader Program Description
## Introduction
The JSON Shader Program Description (JSPD) is a JSON-based format for describing shader programs. It is designed to be a human-readable, machine-readable, and extensible format for describing shader programs. The JSPD is intended to be used as a common format for shader programs across multiple shader languages and shader compilers. The JSPD is designed to be used in conjunction with the Byte Engine Shader Language (BESL).

## Specification
Every node in the JSPD is a JSON object with the same structure. The following is a list of the fields that can be present in a JSPD node:
- `type`: The type of the node. This field is required.

The root node of a JSPD looks like this:
```json
{
	"root": { ... }
}
```

## Samples
### Vertex Shader
```json
{
	"root": {
		"Vertex": {
			"only_under": ["Vertex:Stage"],
			"in_position": {
				"type": "in",
				"in_position": {
					"type": "member",
					"data_type": "vec3f",
				}
			},
			"main": {
				"type": "function",
				"return": "void",
			}
		}
	}
}```