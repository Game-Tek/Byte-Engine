---
icon: bold
---

# Byte Engine Shader Language
## Introduction
BESL is an innovative programming language designed to facilitate the creation of shaders within game engines. Drawing inspiration from the Rust programming language, BESL combines safety, performance, and expressiveness to elevate the art of shader development. This document outlines the key features, syntax, and concepts of BESL for developers seeking a powerful toolset for crafting high-performance graphics in game environments.

BESL features a modern syntax to make shader programming more ergonomic.

Currently, BESL can only compile down to GLSL.

## Code Samples
### Struct declaration
```rust
Light: struct {
	position: vec3f,
	color: vec3f,
	intensity: f32
}
```

### Function declaration
```rust
mod_by_16: fn (x: i32) -> i32 {
	return x % 16
}
```

### Variable declaration
```rust
x: i32 = 5
```