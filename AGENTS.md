# Documentation
Always write struct and trait documentation as:
The `<NAME>` struct/trait <PURPOSE>

Always write about the purpose of the struct/trait or why it wxists instead of what it does.

If functions is larger than a few lines, write a short description of what the function does.

# Errors
When writing errors, first write a succint error message and then a sentence with the most likely cause of the error.

# API Design
- Prefer composition over inheritance. Use traits to define shared behavior and structs to encapsulate data.
- Prefer pure functions. Just inputs and outputs.
- Keep mutability at higher call sites.

# Working
- If I ask you to defer any task, write that into the todo.md file
- Always leave comments for any non-trivial code
