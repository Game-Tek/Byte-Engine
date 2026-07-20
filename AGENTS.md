# Documentation

Use ISO 24495-1 plain-language principles for in-code comments and API documentation. Make the content relevant, findable, understandable, and usable for its intended developer.

Use Microsoft Writing Style Guide principles for user guides and documentation pages. Address the reader directly, lead with the reader's goal, use everyday words, keep the tone helpful, and make the next action clear.

Keep useful intra-code links in in-code and API documentation, even when strict application of ISO 24495-1 would simplify or remove them. Links to related types, traits, functions, methods, modules, and concepts help developers navigate the API.

For workflow-bearing APIs, include a short next-step suggestion that links to the API a developer usually calls or implements next. Add this guidance to modules, types, constructors, and transition methods where it helps developers connect subsystems. Do not add it to trivial fields or accessors.

Always write struct and trait documentation as:
The `<NAME>` struct/trait <PURPOSE>

Always write about the purpose of the struct/trait or why it exists instead of what it does.
Write usage-oriented documentation. How/where will this element be used, instead of this is what it does or how.

If a function is longer than a few lines, write a short description of what the function does.

# Errors

When writing errors, first write a succinct error message and then a sentence with the most likely cause of the error.

# API Design

- Prefer composition over inheritance. Use traits to define shared behavior and structs to encapsulate data.
- Prefer pure functions. Just inputs and outputs.
- Keep mutability at higher call sites.

# Working

- If I ask you to defer any task, write that into the todo.md file
- Always leave comments for any non-trivial code
- Always prefer breaking APIs to ad-hoc changes. The app is not shipped yet. We can make all breaking changes.

# Best Practices

- Avoid allocations, especially transient ones.
