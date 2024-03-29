This section discusses some of the design decisions around rendering.
This ecompasses everything from rendering code to rendering algorithms.

## Actors

### Render orchestrator
The render orchestrator coordinates the rendering of the different render domains.
It manages the global render graph.
It's strictly a piece of the runtime.

### Render system
The render system provides easy to use abstractions over the render backend.
It allows you to create textures, buffers, shaders, etc. and then use them to render things.
It abstracts details like staging buffers, memory allocation, etc. away from the user.

Each render system is backed by a render backend. The render backend is responsible for creating the actual resources and executing the commands.
This render backend can be chosen when creating the render system (e.g. Vulkan, OpenGL, DirectX, etc.)

It also belong to the runtime.

### Render domain
A render domain is collection of renderables that all live in the same space and we'd want to be managed by a common cohesive rendering technique.

The render domain is an interface for defining a rendering environent and the guidelines for generating code for said environment.

### Render model
A render model is an implementation of a render domain.

Say we have a render domain `RenderWorld` we could have a render model `RenderWorldVisibilityBuffer`, `RenderWorldDefered` and `RenderWorldForward`.
Each one would render the world using different techniques.