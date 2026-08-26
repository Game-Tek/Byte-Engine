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

When an error or warning has a documented recovery workflow, include a direct link to the relevant online documentation page. Do not link to a generic documentation landing page when a focused page exists.

# API Design

- Prefer composition over inheritance. Use traits to define shared behavior and structs to encapsulate data.
- Prefer pure functions. Just inputs and outputs.
- Keep mutability at higher call sites.
- Keep dependency injection in mind.
- Separate concerns by layers. E.G: If some feature requires keeping track of occupied and free spaces in a file, write one piece of code that keeps track of that without concerning itself with how to read or modify the file, and the another piece of code that takes the former function's result and applied that to the file. Don't coalesce concerns. 

# Working

- If I ask you to defer any task, write that into the todo.md file
- Always leave comments for any non-trivial code
- Always prefer breaking APIs to ad-hoc changes. The app is not shipped yet. We can make all breaking changes.
- If modifying performance sensitive code, run any existing benchmarks and ensure they do not regress (or they improve) before and after your changes.

# Tests
- Use `cargo nextest run --workspace` as the default test command. Run documentation tests separately with `cargo test --doc --workspace`.
- Tests must exercise contiguous seams. (E.G: don't test material evaluation shaders correctly lower to MSL. MSL lowering is responsability of the BESL MSL shader generator. Material evaluation must only ensure their BESL shader correctly lexes, which is asserted with a BESL lexer test).
- Do not add tombstone tests whose only purpose is to assert that removed code, routes, fields, or features remain absent. Negative tests are appropriate when the failure or absence is itself a current API, security, or persistence contract.
- Test observable behavior and specifications. Write black-box tests that verify the expected behavior of the system. Never write white-box tests that verify internal implementation details. To assert internal implementation details add assertions to the implementation code itself.

# Best Practices

- Avoid allocations, especially transient ones.
